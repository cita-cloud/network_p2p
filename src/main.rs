// Copyright Rivtower Technologies LLC.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod config;

use clap::Clap;
use git_version::git_version;
use log::{debug, info, warn};

const GIT_VERSION: &str = git_version!(
    args = ["--tags", "--always", "--dirty=-modified"],
    fallback = "unknown"
);
const GIT_HOMEPAGE: &str = "https://github.com/rink1969/cita_ng_network";

/// network service
#[derive(Clap)]
#[clap(version = "0.1.0", author = "Rivtower Technologies.")]
struct Opts {
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Clap)]
enum SubCommand {
    /// print information from git
    #[clap(name = "git")]
    GitInfo,
    /// run this service
    #[clap(name = "run")]
    Run(RunOpts),
}

/// A subcommand for run
#[derive(Clap)]
struct RunOpts {
    /// Sets grpc port of config service.
    #[clap(short = "c", long = "config_port", default_value = "49999")]
    config_port: String,
    /// Sets grpc port of this service.
    #[clap(short = "p", long = "port", default_value = "50000")]
    grpc_port: String,
}

fn main() {
    ::std::env::set_var("RUST_BACKTRACE", "full");

    let opts: Opts = Opts::parse();

    match opts.subcmd {
        SubCommand::GitInfo => {
            println!("git version: {}", GIT_VERSION);
            println!("homepage: {}", GIT_HOMEPAGE);
        }
        SubCommand::Run(opts) => {
            // init log4rs
            log4rs::init_file("network-log4rs.yaml", Default::default()).unwrap();
            info!("grpc port of config service: {}", opts.config_port);
            info!("grpc port of this service: {}", opts.grpc_port);
            run(opts);
        }
    }
}

use cita_ng_proto::config::config_service_client::ConfigServiceClient;
use cita_ng_proto::config::{Endpoint, RegisterEndpointInfo, ServiceId};
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;

async fn fetch_config(config_port: String) -> Result<String, Box<dyn std::error::Error>> {
    let config_addr = format!("http://127.0.0.1:{}", config_port);
    let mut client = ConfigServiceClient::connect(config_addr).await?;

    // id of Network service is 0
    let request = Request::new(ServiceId { id: 0 });

    let response = client.get_config(request).await?;

    let config = response.into_inner().config;

    Ok(config)
}

async fn register_endpoint(
    config_port: String,
    port: String,
) -> Result<bool, Box<dyn std::error::Error>> {
    let config_addr = format!("http://127.0.0.1:{}", config_port);
    let mut client = ConfigServiceClient::connect(config_addr).await?;

    // id of Network service is 0
    let request = Request::new(RegisterEndpointInfo {
        id: 0,
        endpoint: Some(Endpoint {
            hostname: "127.0.0.1".to_owned(),
            port,
        }),
    });

    let response = client.register_endpoint(request).await?;

    Ok(response.into_inner().is_success)
}

fn run(opts: RunOpts) {
    let mut rt = Runtime::new().unwrap();
    let config_port = opts.config_port;
    let grpc_port = opts.grpc_port;
    let grpc_port_clone = grpc_port.clone();
    let (config_tx, config_rx) = unbounded();
    let p2p = Arc::new(RwLock::new(None));
    let network_msg_dispatch_table = Arc::new(RwLock::new(HashMap::new()));

    info!("connecting to config service!");
    thread::spawn(move || {
        // register_endpoint first
        loop {
            let ret = rt.block_on(register_endpoint(config_port.clone(), grpc_port.clone()));
            if ret.is_ok() {
                break;
            }
            debug!("register_endpoint failed {:?}", ret);
            thread::sleep(Duration::new(3, 0));
        }
        info!("register endpoint done!");
        let mut old_config_str = String::new();
        loop {
            let ret = rt.block_on(fetch_config(config_port.clone()));
            if let Ok(config_str) = ret {
                if config_str != old_config_str {
                    old_config_str = config_str.clone();
                    info!("get new config!");
                    let _ = config_tx.send(config_str);
                }
            } else {
                debug!("fetch_config failed {:?}", ret);
            }
            thread::sleep(Duration::new(30, 0));
        }
    });

    info!("Start network!");
    let p2p_clone = p2p.clone();
    let network_msg_dispatch_table_clone = network_msg_dispatch_table.clone();
    thread::spawn(move || {
        run_network(config_rx, p2p_clone, network_msg_dispatch_table_clone);
    });

    info!("Start grpc server!");
    let _ = run_grpc_server(grpc_port_clone, p2p, network_msg_dispatch_table);
}

use cita_ng_proto::common::{Empty, SimpleResponse};
use cita_ng_proto::network::{
    network_service_server::NetworkService, network_service_server::NetworkServiceServer,
    NetworkMsg, NetworkStatusResponse, RegisterInfo,
};
use parking_lot::RwLock;
use prost::Message;
use std::collections::HashMap;
use tonic::{transport::Server, Request, Response, Status};

pub struct NetworkServer {
    p2p: Arc<RwLock<Option<P2P>>>,
    network_msg_dispatch_table: Arc<RwLock<HashMap<String, (String, String)>>>,
}

impl NetworkServer {
    fn new(
        p2p: Arc<RwLock<Option<P2P>>>,
        network_msg_dispatch_table: Arc<RwLock<HashMap<String, (String, String)>>>,
    ) -> NetworkServer {
        NetworkServer {
            p2p,
            network_msg_dispatch_table,
        }
    }
}

#[tonic::async_trait]
impl NetworkService for NetworkServer {
    async fn send_msg(
        &self,
        request: Request<NetworkMsg>,
    ) -> Result<Response<SimpleResponse>, Status> {
        debug!("send_msg request: {:?}", request);

        if self.p2p.read().is_some() {
            let msg = request.into_inner();
            let mut buf: Vec<u8> = Vec::new();
            if msg.encode(&mut buf).is_ok() {
                self.p2p
                    .read()
                    .as_ref()
                    .unwrap()
                    .send_message(msg.origin as usize, buf.as_slice());
                let reply = SimpleResponse { is_success: true };
                Ok(Response::new(reply))
            } else {
                Err(Status::internal("encode msg failed"))
            }
        } else {
            Err(Status::internal("network is not ready"))
        }
    }

    async fn broadcast(
        &self,
        request: Request<NetworkMsg>,
    ) -> Result<Response<SimpleResponse>, Status> {
        debug!("broadcast request: {:?}", request);

        if self.p2p.read().is_some() {
            let msg = request.into_inner();
            let mut buf: Vec<u8> = Vec::new();
            if msg.encode(&mut buf).is_ok() {
                self.p2p
                    .read()
                    .as_ref()
                    .unwrap()
                    .broadcast_message(buf.as_slice());
                let reply = SimpleResponse { is_success: true };
                Ok(Response::new(reply))
            } else {
                Err(Status::internal("encode msg failed"))
            }
        } else {
            Err(Status::internal("network is not ready"))
        }
    }

    async fn get_network_status(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<NetworkStatusResponse>, Status> {
        debug!("register_endpoint request: {:?}", request);

        let reply = NetworkStatusResponse { peer_count: 4 };
        Ok(Response::new(reply))
    }

    async fn register_network_msg_handler(
        &self,
        request: Request<RegisterInfo>,
    ) -> Result<Response<SimpleResponse>, Status> {
        debug!("register_network_msg_handler request: {:?}", request);

        let info = request.into_inner();
        let module_name = info.module_name;
        let hostname = info.hostname;
        let port = info.port;

        self.network_msg_dispatch_table
            .write()
            .insert(module_name, (hostname, port));

        let reply = SimpleResponse { is_success: true };
        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn run_grpc_server(
    port: String,
    p2p: Arc<RwLock<Option<P2P>>>,
    network_msg_dispatch_table: Arc<RwLock<HashMap<String, (String, String)>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr_str = format!("127.0.0.1:{}", port);
    let addr = addr_str.parse()?;
    let network_server = NetworkServer::new(p2p, network_msg_dispatch_table);

    Server::builder()
        .add_service(NetworkServiceServer::new(network_server))
        .serve(addr)
        .await?;

    Ok(())
}

use cita_ng_proto::network::network_msg_handler_service_client::NetworkMsgHandlerServiceClient;
use config::NetConfig;
use p2p_simple::{channel::unbounded, channel::Receiver, channel::Select, P2P};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;

async fn dispatch_network_msg(
    port: String,
    msg: NetworkMsg,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest_addr = format!("http://127.0.0.1:{}", port);
    let mut client = NetworkMsgHandlerServiceClient::connect(dest_addr).await?;

    let request = Request::new(msg);

    let _response = client.process_network_msg(request).await?;

    Ok(())
}

fn run_network(
    config_rx: Receiver<String>,
    p2p: Arc<RwLock<Option<P2P>>>,
    network_msg_dispatch_table: Arc<RwLock<HashMap<String, (String, String)>>>,
) {
    let (network_tx, network_rx) = unbounded();

    let mut sel = Select::new();
    let oper_config = sel.recv(&config_rx);
    let oper_network = sel.recv(&network_rx);

    let mut rt = Runtime::new().unwrap();

    loop {
        // Wait until a receive operation becomes ready and try executing it.
        match sel.ready() {
            i if i == oper_config => {
                // got new config
                if let Ok(config_str) = config_rx.try_recv() {
                    if p2p.read().is_some() {
                        let old_p2p = p2p.write().take();
                        old_p2p.unwrap().drop();
                        // wait for p2p's threads to finish
                        thread::sleep(Duration::new(5, 0));
                    }

                    let config = NetConfig::new(&config_str);
                    // init p2p network
                    let listen_addr =
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), config.port);
                    // get peers from config
                    let mut peers = Vec::new();
                    for peer in config.peers {
                        let addr_str = format!("{}:{}", peer.ip, peer.port);
                        peers.push(
                            addr_str
                                .to_socket_addrs()
                                .expect("failed to parse socket address")
                                .next()
                                .unwrap(),
                        );
                    }
                    let new_p2p = Some(P2P::new(
                        config.privkey_path,
                        512 * 1024,
                        listen_addr,
                        peers,
                        network_tx.clone(),
                    ));
                    {
                        *p2p.write() = new_p2p;
                    }
                }
            }
            i if i == oper_network => {
                // got network message
                if let Ok(msg) = network_rx.try_recv() {
                    let (sid, payload) = msg;
                    debug!("received msg {:?} from {}", payload, sid);

                    match NetworkMsg::decode(payload.as_slice()) {
                        Ok(msg) => {
                            if let Some((_hostname, port)) =
                                network_msg_dispatch_table.read().get(&msg.module)
                            {
                                let _ = rt.block_on(dispatch_network_msg(port.to_owned(), msg));
                            }
                        }
                        Err(e) => {
                            warn!("network msg decode failed: {}", e);
                        }
                    }
                } else {
                    warn!("recv network error");
                }
            }
            _ => unreachable!(),
        }
    }
}
