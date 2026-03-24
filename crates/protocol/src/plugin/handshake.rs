use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{CapabilityDescriptor, HandlerDescriptor, PeerDescriptor, ProfileDescriptor};

pub const PROTOCOL_VERSION: &str = "4";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeMessage {
    pub id: String,
    pub protocol_version: String,
    #[serde(default)]
    pub supported_protocol_versions: Vec<String>,
    pub peer: PeerDescriptor,
    #[serde(default)]
    pub capabilities: Vec<CapabilityDescriptor>,
    #[serde(default)]
    pub handlers: Vec<HandlerDescriptor>,
    #[serde(default)]
    pub profiles: Vec<ProfileDescriptor>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResultData {
    pub protocol_version: String,
    pub peer: PeerDescriptor,
    #[serde(default)]
    pub capabilities: Vec<CapabilityDescriptor>,
    #[serde(default)]
    pub handlers: Vec<HandlerDescriptor>,
    #[serde(default)]
    pub profiles: Vec<ProfileDescriptor>,
    #[serde(default)]
    pub metadata: Value,
}
