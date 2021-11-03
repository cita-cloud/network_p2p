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

use crate::config::NetConfig;
use std::fs;
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::rustls::{
    internal::pemfile::{certs, pkcs8_private_keys, rsa_private_keys},
    AllowAnyAuthenticatedClient, Certificate, ClientConfig, KeyLogFile, PrivateKey,
    ProtocolVersion, RootCertStore, ServerConfig, SupportedCipherSuite, ALL_CIPHERSUITES,
};

fn find_suite(name: &str) -> Option<&'static SupportedCipherSuite> {
    for suite in &ALL_CIPHERSUITES {
        let sname = format!("{:?}", suite.suite).to_lowercase();

        if sname == name.to_string().to_lowercase() {
            return Some(suite);
        }
    }

    None
}

fn lookup_suites(suites: &[String]) -> Vec<&'static SupportedCipherSuite> {
    let mut out = Vec::new();

    for csname in suites {
        let scs = find_suite(csname);
        match scs {
            Some(s) => out.push(s),
            None => panic!("cannot look up ciphersuite '{}'", csname),
        }
    }

    out
}

/// Make a vector of protocol versions named in `versions`
fn lookup_versions(versions: &[String]) -> Vec<ProtocolVersion> {
    let mut out = Vec::new();

    for vname in versions {
        let version = match vname.as_ref() {
            "1.2" => ProtocolVersion::TLSv1_2,
            "1.3" => ProtocolVersion::TLSv1_3,
            _ => panic!(
                "cannot look up version '{}', valid are '1.2' and '1.3'",
                vname
            ),
        };
        out.push(version);
    }

    out
}

fn load_certs(filename: &str) -> Vec<Certificate> {
    let certfile = fs::File::open(filename).expect("cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    certs(&mut reader).unwrap()
}

fn load_private_key(filename: &str) -> PrivateKey {
    let rsa_keys = {
        let keyfile = fs::File::open(filename).expect("cannot open private key file");
        let mut reader = BufReader::new(keyfile);
        rsa_private_keys(&mut reader).expect("file contains invalid rsa private key")
    };

    let pkcs8_keys = {
        let keyfile = fs::File::open(filename).expect("cannot open private key file");
        let mut reader = BufReader::new(keyfile);
        pkcs8_private_keys(&mut reader)
            .expect("file contains invalid pkcs8 private key (encrypted keys not supported)")
    };

    // prefer to load pkcs8 keys
    if !pkcs8_keys.is_empty() {
        pkcs8_keys[0].clone()
    } else {
        assert!(!rsa_keys.is_empty());
        rsa_keys[0].clone()
    }
}

fn load_key_and_cert(config: &mut ClientConfig, keyfile: &str, certsfile: &str, cafile: &str) {
    let mut certs = load_certs(certsfile);
    let cacerts = load_certs(cafile);
    let privkey = load_private_key(keyfile);

    // Specially for server.crt not a cert-chain only one server certificate, so manually make
    // a cert-chain.
    if certs.len() == 1 && !cacerts.is_empty() {
        certs.extend(cacerts);
    }

    config
        .set_single_client_cert(certs, privkey)
        .expect("invalid certificate or private key");
}

/// Build a `ServerConfig` from our NetConfig
pub fn make_server_config(config: &NetConfig) -> ServerConfig {
    let cacerts = load_certs(config.ca_cert.as_ref().unwrap());

    let client_auth = {
        let mut client_auth_roots = RootCertStore::empty();
        for cacert in &cacerts {
            client_auth_roots.add(cacert).unwrap();
        }
        AllowAnyAuthenticatedClient::new(client_auth_roots)
    };

    let mut server_config = ServerConfig::new(client_auth);
    server_config.key_log = Arc::new(KeyLogFile::new());

    let mut certs = load_certs(
        config
            .server_chain_certs
            .as_ref()
            .expect("server_chain_certs option missing"),
    );
    let privkey = load_private_key(
        config
            .server_key
            .as_ref()
            .expect("server_key option missing"),
    );

    // Specially for server.crt not a cert-chain only one server certificate, so manually make
    // a cert-chain.
    if certs.len() == 1 && !cacerts.is_empty() {
        certs.extend(cacerts);
    }

    server_config
        .set_single_cert_with_ocsp_and_sct(certs, privkey, vec![], vec![])
        .expect("bad certificates/private key");

    if config.cypher_suits.is_some() {
        server_config.ciphersuites = lookup_suites(
            config
                .cypher_suits
                .as_ref()
                .expect("cypher_suits option error"),
        );
    }

    if config.protocols.is_some() {
        server_config.versions = lookup_versions(config.protocols.as_ref().unwrap());
        server_config.set_protocols(
            &config
                .protocols
                .as_ref()
                .unwrap()
                .iter()
                .map(|proto| proto.as_bytes().to_vec())
                .collect::<Vec<_>>()[..],
        );
    }

    server_config
}

/// Build a `ClientConfig` from our NetConfig
pub fn make_client_config(config: &NetConfig) -> ClientConfig {
    let mut client_config = ClientConfig::new();
    client_config.key_log = Arc::new(KeyLogFile::new());

    if config.cypher_suits.is_some() {
        client_config.ciphersuites = lookup_suites(config.cypher_suits.as_ref().unwrap());
    }

    if config.protocols.is_some() {
        client_config.versions = lookup_versions(config.protocols.as_ref().unwrap());

        client_config.set_protocols(
            &config
                .protocols
                .as_ref()
                .unwrap()
                .iter()
                .map(|proto| proto.as_bytes().to_vec())
                .collect::<Vec<_>>()[..],
        );
    }

    let cafile = config.ca_cert.as_ref().unwrap();

    let certfile = fs::File::open(cafile).expect("Cannot open CA file");
    let mut reader = BufReader::new(certfile);
    client_config.root_store.add_pem_file(&mut reader).unwrap();

    if config.server_key.is_some() || config.server_chain_certs.is_some() {
        load_key_and_cert(
            &mut client_config,
            config
                .server_key
                .as_ref()
                .expect("must provide client_key with client_cert"),
            config
                .server_chain_certs
                .as_ref()
                .expect("must provide client_cert with client_key"),
            cafile,
        );
    }

    client_config
}
