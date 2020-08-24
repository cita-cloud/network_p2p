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
const GIT_HOMEPAGE: &str = "https://github.com/cita-cloud/network_p2p";

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
    /// Sets grpc port of this service.
    #[clap(short = 'p', long = "port", default_value = "50000")]
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
            info!("grpc port of this service: {}", opts.grpc_port);
            run(opts);
        }
    }
}

use cita_cloud_proto::common::{Empty, SimpleResponse};
use cita_cloud_proto::network::{
    network_service_server::NetworkService, network_service_server::NetworkServiceServer,
    NetworkMsg, NetworkStatusResponse, RegisterInfo,
};
use prost::Message;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tonic::{transport::Server, Request, Response, Status};

use cita_cloud_proto::network::network_msg_handler_service_client::NetworkMsgHandlerServiceClient;
use config::NetConfig;
use p2p_simple::{channel::unbounded, channel::Receiver, P2P};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;

#[tokio::main]
async fn run(opts: RunOpts) {
    let grpc_port = opts.grpc_port;

    //read network-config.toml
    let buffer = std::fs::read_to_string("network-config.toml")
        .unwrap_or_else(|err| panic!("Error while loading config: [{}]", err));
    let config = NetConfig::new(&buffer);

    // init p2p network
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), config.port);
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

    let (network_tx, network_rx) = unbounded();
    let p2p = P2P::new(
        "network-key".to_owned(),
        512 * 1024,
        listen_addr,
        peers,
        network_tx,
    );

    let network_msg_dispatch_table = Arc::new(RwLock::new(HashMap::new()));

    info!("Start network!");
    let network_msg_dispatch_table_clone = network_msg_dispatch_table.clone();
    tokio::spawn(run_network(network_rx, network_msg_dispatch_table_clone));

    info!("Start grpc server!");
    let _ = run_grpc_server(grpc_port, p2p, network_msg_dispatch_table).await;
}

pub struct NetworkServer {
    p2p: P2P,
    network_msg_dispatch_table: Arc<RwLock<HashMap<String, (String, String)>>>,
}

impl NetworkServer {
    fn new(
        p2p: P2P,
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

        let msg = request.into_inner();
        let mut buf: Vec<u8> = Vec::new();
        if msg.encode(&mut buf).is_ok() {
            self.p2p.send_message(msg.origin as usize, buf.as_slice());
            let reply = SimpleResponse { is_success: true };
            Ok(Response::new(reply))
        } else {
            Err(Status::internal("encode msg failed"))
        }
    }

    async fn broadcast(
        &self,
        request: Request<NetworkMsg>,
    ) -> Result<Response<SimpleResponse>, Status> {
        debug!("broadcast request: {:?}", request);

        let msg = request.into_inner();
        let mut buf: Vec<u8> = Vec::new();
        if msg.encode(&mut buf).is_ok() {
            self.p2p.broadcast_message(buf.as_slice());
            let reply = SimpleResponse { is_success: true };
            Ok(Response::new(reply))
        } else {
            Err(Status::internal("encode msg failed"))
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

        let mut network_msg_dispatch_table = self.network_msg_dispatch_table.write().await;
        network_msg_dispatch_table.insert(module_name, (hostname, port));

        let reply = SimpleResponse { is_success: true };
        Ok(Response::new(reply))
    }
}

async fn run_grpc_server(
    port: String,
    p2p: P2P,
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

async fn dispatch_network_msg(
    client_map: Arc<
        RwLock<HashMap<String, NetworkMsgHandlerServiceClient<tonic::transport::Channel>>>,
    >,
    port: String,
    msg: NetworkMsg,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = client_map.read().await.get(&port).cloned();

    if client.is_none() {
        let dest_addr = format!("http://127.0.0.1:{}", port);
        let c = NetworkMsgHandlerServiceClient::connect(dest_addr).await?;
        client_map.write().await.insert(port, c.clone());
        client.replace(c);
    }

    let mut client = client.unwrap();
    let request = Request::new(msg);
    let _response = client.process_network_msg(request).await?;

    Ok(())
}

async fn run_network(
    network_rx: Receiver<(usize, Vec<u8>)>,
    network_msg_dispatch_table: Arc<RwLock<HashMap<String, (String, String)>>>,
) {
    let client_map = Arc::new(RwLock::new(HashMap::<
        String,
        NetworkMsgHandlerServiceClient<tonic::transport::Channel>,
    >::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, NetworkMsg)>();
    std::thread::spawn(move || loop {
        if let Ok(msg) = network_rx.recv() {
            let (sid, payload) = msg;
            debug!("received msg {:?} from {}", payload, sid);

            match NetworkMsg::decode(payload.as_slice()) {
                Ok(msg) => {
                    tx.send((sid, msg)).expect("send (sid, msg) failed");
                }
                Err(e) => {
                    warn!("network msg decode failed: {}", e);
                }
            }
        }
    });
    while let Some((sid, mut msg)) = rx.recv().await {
        msg.origin = sid as u64;
        let dispatch_table = network_msg_dispatch_table.clone();
        let client_map = client_map.clone();
        tokio::spawn(async move {
            let port = {
                let table = dispatch_table.read().await;
                if let Some((_hostname, port)) = table.get(&msg.module) {
                    port.to_owned()
                } else {
                    "".to_owned()
                }
            };
            if !port.is_empty() {
                let _ = dispatch_network_msg(client_map, port, msg).await;
            }
        });
    }
}
