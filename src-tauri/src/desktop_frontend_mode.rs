#![cfg_attr(not(test), allow(dead_code))]

#[allow(dead_code)]
pub const FRONTEND_MODE_ENV: &str = "ASTRCODE_DESKTOP_FRONTEND_MODE";
pub const TAURI_CLI_VERBOSITY_ENV: &str = "TAURI_CLI_VERBOSITY";
pub const DEP_TAURI_DEV_ENV: &str = "DEP_TAURI_DEV";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopFrontendMode {
    TauriDevCli,
    PlainCargo,
    Packaged,
}

impl DesktopFrontendMode {
    #[allow(dead_code)]
    pub const fn as_env_value(self) -> &'static str {
        match self {
            Self::TauriDevCli => "tauri-dev-cli",
            Self::PlainCargo => "plain-cargo",
            Self::Packaged => "packaged",
        }
    }

    pub fn parse(mode: &str) -> Result<Self, String> {
        match mode {
            "tauri-dev-cli" => Ok(Self::TauriDevCli),
            "plain-cargo" => Ok(Self::PlainCargo),
            "packaged" => Ok(Self::Packaged),
            other => Err(format!("unknown AstrCode desktop frontend mode '{other}'")),
        }
    }

    pub fn resolve(tauri_cli_invoked: bool, tauri_is_dev: bool) -> Self {
        match (tauri_cli_invoked, tauri_is_dev) {
            (true, true) => Self::TauriDevCli,
            (true, false) => Self::Packaged,
            (false, _) => Self::PlainCargo,
        }
    }
}

pub fn tauri_cli_invoked_from_env() -> bool {
    std::env::var_os(TAURI_CLI_VERBOSITY_ENV).is_some()
}

pub fn tauri_is_dev_from_env() -> bool {
    matches!(
        std::env::var(DEP_TAURI_DEV_ENV).as_deref(),
        Ok("true") | Ok("1")
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DEP_TAURI_DEV_ENV, DesktopFrontendMode, TAURI_CLI_VERBOSITY_ENV,
        tauri_cli_invoked_from_env, tauri_is_dev_from_env,
    };

    #[test]
    fn parse_accepts_supported_modes() {
        assert_eq!(
            DesktopFrontendMode::parse("tauri-dev-cli"),
            Ok(DesktopFrontendMode::TauriDevCli)
        );
        assert_eq!(
            DesktopFrontendMode::parse("plain-cargo"),
            Ok(DesktopFrontendMode::PlainCargo)
        );
        assert_eq!(
            DesktopFrontendMode::parse("packaged"),
            Ok(DesktopFrontendMode::Packaged)
        );
    }

    #[test]
    fn resolve_maps_build_contexts_to_frontend_modes() {
        assert_eq!(
            DesktopFrontendMode::resolve(false, true),
            DesktopFrontendMode::PlainCargo
        );
        assert_eq!(
            DesktopFrontendMode::resolve(false, false),
            DesktopFrontendMode::PlainCargo
        );
        assert_eq!(
            DesktopFrontendMode::resolve(true, true),
            DesktopFrontendMode::TauriDevCli
        );
        assert_eq!(
            DesktopFrontendMode::resolve(true, false),
            DesktopFrontendMode::Packaged
        );
    }

    #[test]
    fn env_helpers_detect_tauri_cli_and_dev_mode() {
        let original_cli = std::env::var_os(TAURI_CLI_VERBOSITY_ENV);
        let original_dev = std::env::var_os(DEP_TAURI_DEV_ENV);

        std::env::remove_var(TAURI_CLI_VERBOSITY_ENV);
        std::env::remove_var(DEP_TAURI_DEV_ENV);
        assert!(!tauri_cli_invoked_from_env());
        assert!(!tauri_is_dev_from_env());

        std::env::set_var(TAURI_CLI_VERBOSITY_ENV, "0");
        std::env::set_var(DEP_TAURI_DEV_ENV, "true");
        assert!(tauri_cli_invoked_from_env());
        assert!(tauri_is_dev_from_env());

        std::env::set_var(DEP_TAURI_DEV_ENV, "false");
        assert!(!tauri_is_dev_from_env());

        match original_cli {
            Some(value) => std::env::set_var(TAURI_CLI_VERBOSITY_ENV, value),
            None => std::env::remove_var(TAURI_CLI_VERBOSITY_ENV),
        }
        match original_dev {
            Some(value) => std::env::set_var(DEP_TAURI_DEV_ENV, value),
            None => std::env::remove_var(DEP_TAURI_DEV_ENV),
        }
    }
}
