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

use log::{debug, warn};
use tentacle::{
    builder::{MetaBuilder, ServiceBuilder},
    context::{ProtocolContext, ProtocolContextMutRef, ServiceContext},
    error::DialerErrorKind,
    multiaddr::MultiAddr,
    service::{
        ProtocolHandle, ServiceAsyncControl, ServiceError, ServiceEvent,
        SessionType, TargetProtocol, TargetSession,
    },
    traits::{ServiceHandle, ServiceProtocol},
    ProtocolId, SessionId,
};

use channel::Sender;
pub use crossbeam_channel as channel;

use parking_lot::RwLock;

use tokio_util::codec::LengthDelimitedCodec;

use crate::config::NetConfig;
use status_code::StatusCode;
use std::collections::HashMap;
use std::str;
use std::sync::Arc;
use std::time::Duration;
use tentacle::service::TlsConfig;
use crate::util::{make_server_config, make_client_config};

pub const PROTOCOL_ID: ProtocolId = ProtocolId::new(0);

#[derive(Clone)]
struct PeersManager {
    outbound_peers: Arc<RwLock<HashMap<usize, MultiAddr>>>,
}

impl PeersManager {
    fn check_addr_exists(&self, addr: &MultiAddr) -> bool {
        for (_, v) in self.outbound_peers.read().iter() {
            if v == addr {
                return true;
            }
        }
        false
    }

    fn add_peer(&self, session_id: usize, peer: &MultiAddr) {
        self.outbound_peers.write().insert(session_id, peer.clone());
    }

    fn remove_peer(&self, session_id: usize) {
        let _ = self.outbound_peers.write().remove_entry(&session_id);
    }

    fn peer_count(&self) -> u64 {
        self.outbound_peers.read().len() as u64
    }

    fn get_peers(&self) -> Vec<(usize, MultiAddr)> {
        let mut infos = Vec::new();
        for (origin, add) in self.outbound_peers.read().iter() {
            infos.push((*origin, add.clone()))
        }
        infos
    }
}

struct PHandle {
    tx: Sender<(usize, Vec<u8>)>,
    peers_manager: PeersManager,
}

#[tentacle::async_trait]
impl ServiceProtocol for PHandle {
    async fn init(&mut self, context: &mut ProtocolContext) {
        debug!("init protocol {}", context.proto_id);
    }

    async fn connected(&mut self, context: ProtocolContextMutRef<'_>, version: &str) {
        let session = context.session;
        debug!(
            "proto id [{}] open on session [{}], address: [{}], type: [{:?}], version: {}",
            context.proto_id, session.id, session.address, session.ty, version
        );
        if session.ty == SessionType::Outbound {
            self.peers_manager
                .add_peer(session.id.value(), &session.address);
        }
    }

    async fn disconnected(&mut self, context: ProtocolContextMutRef<'_>) {
        debug!(
            "proto id [{}] close on session [{}]",
            context.proto_id, context.session.id
        );
        self.peers_manager.remove_peer(context.session.id.value());
    }

    async fn received(&mut self, context: ProtocolContextMutRef<'_>, data: bytes::Bytes) {
        debug!(
            "received from [{}]: proto [{}] data len {} data {:?}",
            context.session.id,
            context.proto_id,
            data.len(),
            data
        );
        self.tx
            .send((context.session.id.value(), data.as_ref().to_vec()))
            .unwrap();
    }

    async fn notify(&mut self, context: &mut ProtocolContext, token: u64) {
        debug!(
            "proto [{}] received notify token: {}",
            context.proto_id, token
        );
    }
}

struct SHandle {
    peers_manager: PeersManager,
}

#[tentacle::async_trait]
impl ServiceHandle for SHandle {
    async fn handle_error(&mut self, _context: &mut ServiceContext, error: ServiceError) {
        match error {
            ServiceError::DialerError { address, error } => {
                // If dial to a connected node, need add it to connected address list.
                match error {
                    DialerErrorKind::RepeatedConnection(session_id) => {
                        self.peers_manager.add_peer(session_id.value(), &address);
                    }
                    _ => {
                        warn!("service error: {:?}", error);
                    }
                }
            }
            _ => {
                warn!("service error: {:?}", error);
            }
        }
    }
    async fn handle_event(&mut self, _context: &mut ServiceContext, event: ServiceEvent) {
        debug!("service event: {:?}", event);
    }
}

#[derive(Clone)]
pub struct P2P {
    peers_manager: PeersManager,
    service_control: ServiceAsyncControl,
}

impl P2P {
    pub fn new(
        config: NetConfig,
        listen_addr: MultiAddr,
        peer_addrs: Vec<MultiAddr>,
        tx: Sender<(usize, Vec<u8>)>,
    ) -> P2P {
        // shared peers manager struct
        let peers_manager = PeersManager {
            outbound_peers: Arc::new(RwLock::new(HashMap::new())),
        };

        // create meta of protocol to transfer
        let peers_manager_clone = peers_manager.clone();
        let grpc_frame = config.grpc_frame;
        let meta = MetaBuilder::new()
            .id(PROTOCOL_ID)
            .codec(move || {
                let mut lcodec = LengthDelimitedCodec::new();
                lcodec.set_max_frame_length(grpc_frame);
                Box::new(lcodec)
            })
            .service_handle(move || {
                let handle = Box::new(PHandle {
                    tx: tx.clone(),
                    peers_manager: peers_manager_clone,
                });
                ProtocolHandle::Callback(handle)
            })
            .build();

        let mut service_cfg = ServiceBuilder::default()
            .insert_protocol(meta)
            .forever(true);

        let peers_manager_clone = peers_manager.clone();

        if config.enable_tls {
            log::info!("Network use tls");
            let tls_config = TlsConfig::new(
                Some(make_server_config(&config)),
                Some(make_client_config(&config)),
            );
            service_cfg = service_cfg
                .tls_config(tls_config)
        };

        // todo discovery protocol
        if config.enable_discovery {

        }

        let mut service = service_cfg
            .build(SHandle {
                peers_manager: peers_manager_clone,
            });

        let service_control = service.control().clone();

        let peers_manager_clone = peers_manager.clone();

        let (addr_sender, addr_receiver) = channel::bounded(1);

        // Start a dedicated thread for the tokio runtime.
        // Do not use `tokio::spawn` directly, which requires the user to run it in a tokio runtime.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let addr = service.listen(listen_addr).await.unwrap();

                addr_sender.send(addr).unwrap();

                service.run().await;
            });
        });

        addr_receiver.recv().unwrap();

        let controller = service_control.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(300));
                loop {
                    interval.tick().await;
                    for addr in peer_addrs.clone() {
                        if !peers_manager_clone.check_addr_exists(&addr) {
                            controller.dial(addr, TargetProtocol::All).await.unwrap();
                        }
                    }
                }
            })
        });

        P2P {
            peers_manager,
            service_control,
        }
    }

    pub async fn send_message(&self, session_id: usize, message: &[u8]) -> StatusCode {
        match self
            .service_control
            .send_message_to(
                SessionId::new(session_id),
                PROTOCOL_ID,
                bytes::Bytes::copy_from_slice(message),
            )
            .await
        {
            Err(e) => {
                warn!("send_message_to: session({}) failed: {:?}", session_id, e);
                StatusCode::SendMsgError
            }
            Ok(()) => StatusCode::Success,
        }
    }

    pub async fn broadcast_message(&self, message: &[u8]) -> StatusCode {
        match self
            .service_control
            .filter_broadcast(
                TargetSession::All,
                PROTOCOL_ID,
                bytes::Bytes::copy_from_slice(message),
            )
            .await
        {
            Err(e) => {
                warn!("broadcast_message: failed: {:?}", e);
                StatusCode::BroadcastMsgError
            }
            Ok(()) => StatusCode::Success,
        }
    }

    pub fn get_peer_count(&self) -> u64 {
        self.peers_manager.peer_count()
    }

    pub async fn add_node(&self, addr: MultiAddr) -> StatusCode {
        match self.service_control.dial(addr, TargetProtocol::All).await {
            Ok(()) => StatusCode::Success,
            Err(e) => {
                warn!("add_node error: {:?}", e);
                StatusCode::DialNodeFail
            }
        }
    }

    pub fn get_peers_info(&self) -> Vec<(usize, MultiAddr)> {
        self.peers_manager.get_peers()
    }
}
