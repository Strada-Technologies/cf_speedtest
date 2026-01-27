use std::{sync::Arc, time::Duration};
use ureq::Agent;

use crate::{CONNECT_TIMEOUT_MILLIS, OUR_USER_AGENT};

pub fn create_configured_agent() -> Agent {
    let provider = rustls::crypto::aws_lc_rs::default_provider();

    let tls_config = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(Arc::new(provider))
        .build();

    let agent_config = Agent::config_builder()
        .tls_config(tls_config)
        .timeout_connect(Some(Duration::from_millis(CONNECT_TIMEOUT_MILLIS)))
        .user_agent(OUR_USER_AGENT)
        .build();

    agent_config.into()
}
