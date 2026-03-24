use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentModelInfoDto {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptionDto {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}
