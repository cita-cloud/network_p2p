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
mod p2p;
mod panic_hook;
mod util;

use crate::panic_hook::set_panic_handler;
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
    /// Chain config path
    #[clap(short = 'c', long = "config", default_value = "config.toml")]
    config_path: String,
}

fn main() {
    ::std::env::set_var("RUST_BACKTRACE", "full");
    set_panic_handler();

    let opts: Opts = Opts::parse();

    match opts.subcmd {
        SubCommand::GitInfo => {
            println!("git version: {}", GIT_VERSION);
            println!("homepage: {}", GIT_HOMEPAGE);
        }
        SubCommand::Run(opts) => {
            let fin = run(opts);
            warn!("Should not reach here {:?}", fin);
        }
    }
}

use cita_cloud_proto::network::{
    network_service_server::NetworkService, network_service_server::NetworkServiceServer,
    NetworkMsg, NetworkStatusResponse, RegisterInfo,
};
use prost::Message;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tonic::{transport::Server, Request, Response, Status};

use cita_cloud_proto::common::{Empty, NodeNetInfo, TotalNodeNetInfo};
use cita_cloud_proto::network::network_msg_handler_service_client::NetworkMsgHandlerServiceClient;
use config::NetConfig;
use p2p::{channel::unbounded, channel::Receiver, P2P};
use status_code::StatusCode;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::str::FromStr;
use std::sync::Arc;
use tentacle::multiaddr::MultiAddr;

#[tokio::main]
async fn run(opts: RunOpts) -> Result<(), StatusCode> {
    let config = NetConfig::new(&opts.config_path);
    // init log4rs
    log4rs::init_file(&config.log_file, Default::default()).unwrap();

    let grpc_port = {
        if "50000" != opts.grpc_port {
            opts.grpc_port.clone()
        } else if config.grpc_port != 50000 {
            config.grpc_port.to_string()
        } else {
            "50000".to_string()
        }
    };
    info!("grpc port of this service: {}", grpc_port);

    let listen_addr = {
        if config.enable_tls {
            format!("/ip4/0.0.0.0/tcp/{}/tls/{}", config.port, &config.domain)
        } else {
            format!("/ip4/0.0.0.0/tcp/{}", config.port)
        }
    };

    // init p2p network
    let listen_addr = MultiAddr::from_str(&listen_addr).map_err(|e| {
        warn!("parse listen multi-addr({}) failed", &listen_addr);
        StatusCode::MultiAddrParseError
    })?;
    // get peers from config
    let mut peers = Vec::new();
    for peer in &config.peers {
        peers.push(peer.address.clone());
    }

    let (network_tx, network_rx) = unbounded();
    let p2p = P2P::new(config, listen_addr, peers, network_tx);

    let network_msg_dispatch_table = Arc::new(RwLock::new(HashMap::new()));

    info!("Start network!");
    let network_msg_dispatch_table_clone = network_msg_dispatch_table.clone();
    tokio::spawn(run_network(network_rx, network_msg_dispatch_table_clone));

    info!("Start grpc server!");
    let _ = run_grpc_server(grpc_port, p2p, network_msg_dispatch_table).await;

    Ok(())
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
    ) -> Result<Response<cita_cloud_proto::common::StatusCode>, Status> {
        debug!("send_msg request: {:?}", request);

        let msg = request.into_inner();
        let mut buf: Vec<u8> = Vec::new();
        if msg.encode(&mut buf).is_ok() {
            let status = self
                .p2p
                .send_message(msg.origin as usize, buf.as_slice())
                .await;
            Ok(Response::new(status.into()))
        } else {
            warn!("send_msg: encode NetworkMsg failed");
            Ok(Response::new(StatusCode::EncodeError.into()))
        }
    }

    async fn broadcast(
        &self,
        request: Request<NetworkMsg>,
    ) -> Result<Response<cita_cloud_proto::common::StatusCode>, Status> {
        debug!("broadcast request: {:?}", request);

        let msg = request.into_inner();
        let mut buf: Vec<u8> = Vec::new();
        if msg.encode(&mut buf).is_ok() {
            let status = self.p2p.broadcast_message(buf.as_slice()).await;
            Ok(Response::new(status.into()))
        } else {
            warn!("broadcast: encode NetworkMsg failed");
            Ok(Response::new(StatusCode::EncodeError.into()))
        }
    }

    async fn get_network_status(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<NetworkStatusResponse>, Status> {
        debug!("get_network_status request: {:?}", request);

        Ok(Response::new(NetworkStatusResponse {
            peer_count: self.p2p.get_peer_count(),
        }))
    }

    async fn register_network_msg_handler(
        &self,
        request: Request<RegisterInfo>,
    ) -> Result<Response<cita_cloud_proto::common::StatusCode>, Status> {
        debug!("register_network_msg_handler request: {:?}", request);

        let info = request.into_inner();
        let module_name = info.module_name;
        let hostname = info.hostname;
        let port = info.port;

        let mut network_msg_dispatch_table = self.network_msg_dispatch_table.write().await;
        network_msg_dispatch_table.insert(module_name, (hostname, port));

        Ok(Response::new(StatusCode::Success.into()))
    }

    async fn add_node(
        &self,
        request: Request<NodeNetInfo>,
    ) -> Result<Response<cita_cloud_proto::common::StatusCode>, Status> {
        debug!("add_node request: {:?}", request);

        let info = request.into_inner();
        if let Ok(addr) = MultiAddr::from_str(&info.multi_address) {
            Ok(Response::new(self.p2p.add_node(addr).await.into()))
        } else {
            warn!("add_node: parse multi-addr({}) failed", &info.multi_address);
            Ok(Response::new(StatusCode::MultiAddrParseError.into()))
        }
    }

    async fn get_peers_net_info(
        &self,
        _: Request<Empty>,
    ) -> Result<Response<TotalNodeNetInfo>, Status> {
        debug!("get_peers_net_info request");

        let infos = self.p2p.get_peers_info();
        let mut nodes = Vec::new();

        for info in infos {
            nodes.push(NodeNetInfo {
                multi_address: info.1.to_string(),
                origin: info.0 as u64,
            })
        }

        Ok(Response::new(TotalNodeNetInfo { nodes }))
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
