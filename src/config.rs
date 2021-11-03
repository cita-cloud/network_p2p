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

use cloud_util::common::read_toml;
use serde_derive::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct NetConfig {
    pub domain: String,

    pub port: u16,

    pub grpc_port: u16,

    pub peers: Vec<PeerConfig>,

    pub grpc_frame: usize,

    pub enable_tls: bool,

    pub server_chain_certs: Option<String>,

    pub server_key: Option<String>,

    pub ca_cert: Option<String>,

    pub protocols: Option<Vec<String>>,

    pub cypher_suits: Option<Vec<String>>,

    pub enable_discovery: bool,

    pub log_file: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct PeerConfig {
    pub address: String,
}

impl Default for PeerConfig {
    fn default() -> Self {
        Self {
            address: "".to_string(),
        }
    }
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            enable_tls: false,
            domain: "".to_string(),
            port: 40000,
            grpc_port: 50000,
            peers: vec![],
            grpc_frame: (1 << 24) - 1,
            server_chain_certs: None,
            server_key: None,
            ca_cert: None,
            protocols: None,
            cypher_suits: None,
            enable_discovery: false,
            log_file: "network-log4rs.yaml".to_string(),
        }
    }
}

impl NetConfig {
    pub fn new(config_str: &str) -> Self {
        read_toml(config_str, "network_p2p")
    }
}

#[cfg(test)]
mod tests {
    use super::NetConfig;

    #[test]
    fn basic_test() {
        let config = NetConfig::new("example/config.toml");

        assert_eq!(config.grpc_port, 60005);
    }
}
