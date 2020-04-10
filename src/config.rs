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

use serde_derive::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct NetConfig {
    pub port: u16,
    pub peers: Vec<PeerConfig>,
    pub privkey_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PeerConfig {
    pub ip: String,
    pub port: u16,
}

impl NetConfig {
    pub fn new(config_str: &str) -> Self {
        toml::from_str::<NetConfig>(config_str).expect("Error while parsing config")
    }
}

#[cfg(test)]
mod tests {
    use super::NetConfig;

    #[test]
    fn basic_test() {
        let toml_str = r#"
        port = 40000
        privkey_path = "0_privkey"
        [[peers]]
            ip = "127.0.0.1"
            port = 40001
        [[peers]]
            ip = "127.0.0.1"
            port = 40002
        "#;

        let config = NetConfig::new(toml_str);

        assert_eq!(config.port, 40000);
        assert_eq!(config.privkey_path, "0_privkey".to_owned());
        assert_eq!(config.peers.len(), 2);
    }
}
