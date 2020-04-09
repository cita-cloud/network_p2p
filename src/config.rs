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
use std::fs::File;
use std::io::Read;

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
    pub fn new(path: &str) -> Self {
        let mut buffer = String::new();
        File::open(path)
            .and_then(|mut f| f.read_to_string(&mut buffer))
            .unwrap_or_else(|err| panic!("Error while loading config: [{}]", err));
        toml::from_str::<NetConfig>(&buffer).expect("Error while parsing config")
    }
}

#[cfg(test)]
mod tests {
    use super::NetConfig;
    use std::io::Write;
    use tempfile::NamedTempFile;

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

        let mut tmp_file: NamedTempFile = NamedTempFile::new().unwrap();
        tmp_file.write_all(toml_str.as_bytes()).unwrap();
        let path = tmp_file.path().to_str().unwrap();
        let config = NetConfig::new(path);

        assert_eq!(config.port, 40000);
        assert_eq!(config.privkey_path, "0_privkey".to_owned());
        assert_eq!(config.peers.len(), 2);
    }
}
