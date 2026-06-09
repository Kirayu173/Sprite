use experimental_api_macros::ExperimentalApi;
use runtime_protocol::config_types::ApprovalsReviewer as CoreApprovalsReviewer;
use runtime_protocol::config_types::SandboxMode as CoreSandboxMode;
use runtime_protocol::protocol::AskForApproval as CoreAskForApproval;
use runtime_protocol::protocol::GranularApprovalConfig as CoreGranularApprovalConfig;
use runtime_protocol::protocol::NonSteerableTurnKind as CoreNonSteerableTurnKind;
use runtime_protocol::protocol::RuntimeErrorInfo as CoreRuntimeErrorInfo;
use schemars::JsonSchema;
use schemars::r#gen::SchemaGenerator;
use schemars::schema::InstanceType;
use schemars::schema::Metadata;
use schemars::schema::Schema;
use schemars::schema::SchemaObject;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use ts_rs::TS;

// Macro to declare a camelCased API v2 enum mirroring a core enum which
// tends to use either snake_case or kebab-case.
macro_rules! v2_enum_from_core {
    (
        $(#[$enum_meta:meta])*
        pub enum $Name:ident from $Src:path {
            $( $(#[$variant_meta:meta])* $Variant:ident ),+ $(,)?
        }
    ) => {
        #[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
        $(#[$enum_meta])*
        #[serde(rename_all = "camelCase")]
        #[ts(export_to = "v2/")]
        pub enum $Name {
            $( $(#[$variant_meta])* $Variant ),+
        }

        impl $Name {
            pub fn to_core(self) -> $Src {
                match self { $( $Name::$Variant => <$Src>::$Variant ),+ }
            }
        }

        impl From<$Src> for $Name {
            fn from(value: $Src) -> Self {
                match value { $( <$Src>::$Variant => $Name::$Variant ),+ }
            }
        }
    };
}

pub(super) use v2_enum_from_core;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum NonSteerableTurnKind {
    Review,
    Compact,
}

/// This translation layer makes sure that we expose runtime error codes in camel case.
///
/// When an upstream HTTP status is available from a provider,
/// it is forwarded in `httpStatusCode` on the relevant `RuntimeErrorInfo` variant.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum RuntimeErrorInfo {
    ContextWindowExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    ProviderPolicy,
    HttpConnectionFailed {
        #[serde(rename = "httpStatusCode")]
        #[ts(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    /// Failed to connect to the response SSE stream.
    ResponseStreamConnectionFailed {
        #[serde(rename = "httpStatusCode")]
        #[ts(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    InternalServerError,
    Unauthorized,
    BadRequest,
    ThreadRollbackFailed,
    SandboxError,
    /// The response SSE stream disconnected in the middle of a turn before completion.
    ResponseStreamDisconnected {
        #[serde(rename = "httpStatusCode")]
        #[ts(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    /// Reached the retry limit for responses.
    ResponseTooManyFailedAttempts {
        #[serde(rename = "httpStatusCode")]
        #[ts(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    /// Returned when `turn/start` or `turn/steer` is submitted while the current active turn
    /// cannot accept same-turn steering, for example `/review` or manual `/compact`.
    ActiveTurnNotSteerable {
        #[serde(rename = "turnKind")]
        #[ts(rename = "turnKind")]
        turn_kind: NonSteerableTurnKind,
    },
    Other,
}

impl From<CoreRuntimeErrorInfo> for RuntimeErrorInfo {
    fn from(value: CoreRuntimeErrorInfo) -> Self {
        match value {
            CoreRuntimeErrorInfo::ContextWindowExceeded => RuntimeErrorInfo::ContextWindowExceeded,
            CoreRuntimeErrorInfo::UsageLimitExceeded => RuntimeErrorInfo::UsageLimitExceeded,
            CoreRuntimeErrorInfo::ServerOverloaded => RuntimeErrorInfo::ServerOverloaded,
            CoreRuntimeErrorInfo::ProviderPolicy => RuntimeErrorInfo::ProviderPolicy,
            CoreRuntimeErrorInfo::HttpConnectionFailed { http_status_code } => {
                RuntimeErrorInfo::HttpConnectionFailed { http_status_code }
            }
            CoreRuntimeErrorInfo::ResponseStreamConnectionFailed { http_status_code } => {
                RuntimeErrorInfo::ResponseStreamConnectionFailed { http_status_code }
            }
            CoreRuntimeErrorInfo::InternalServerError => RuntimeErrorInfo::InternalServerError,
            CoreRuntimeErrorInfo::Unauthorized => RuntimeErrorInfo::Unauthorized,
            CoreRuntimeErrorInfo::BadRequest => RuntimeErrorInfo::BadRequest,
            CoreRuntimeErrorInfo::ThreadRollbackFailed => RuntimeErrorInfo::ThreadRollbackFailed,
            CoreRuntimeErrorInfo::SandboxError => RuntimeErrorInfo::SandboxError,
            CoreRuntimeErrorInfo::ResponseStreamDisconnected { http_status_code } => {
                RuntimeErrorInfo::ResponseStreamDisconnected { http_status_code }
            }
            CoreRuntimeErrorInfo::ResponseTooManyFailedAttempts { http_status_code } => {
                RuntimeErrorInfo::ResponseTooManyFailedAttempts { http_status_code }
            }
            CoreRuntimeErrorInfo::ActiveTurnNotSteerable { turn_kind } => {
                RuntimeErrorInfo::ActiveTurnNotSteerable {
                    turn_kind: turn_kind.into(),
                }
            }
            CoreRuntimeErrorInfo::Other => RuntimeErrorInfo::Other,
        }
    }
}

impl From<CoreNonSteerableTurnKind> for NonSteerableTurnKind {
    fn from(value: CoreNonSteerableTurnKind) -> Self {
        match value {
            CoreNonSteerableTurnKind::Review => Self::Review,
            CoreNonSteerableTurnKind::Compact => Self::Compact,
        }
    }
}

#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS, ExperimentalApi,
)]
#[serde(rename_all = "kebab-case")]
#[ts(rename_all = "kebab-case", export_to = "v2/")]
pub enum AskForApproval {
    #[serde(rename = "untrusted")]
    #[ts(rename = "untrusted")]
    UnlessTrusted,
    OnFailure,
    OnRequest,
    #[experimental("askForApproval.granular")]
    Granular {
        sandbox_approval: bool,
        rules: bool,
        #[serde(default)]
        skill_approval: bool,
        #[serde(default)]
        request_permissions: bool,
        mcp_elicitations: bool,
    },
    Never,
}

impl AskForApproval {
    pub fn to_core(self) -> CoreAskForApproval {
        match self {
            AskForApproval::UnlessTrusted => CoreAskForApproval::UnlessTrusted,
            AskForApproval::OnFailure => CoreAskForApproval::OnFailure,
            AskForApproval::OnRequest => CoreAskForApproval::OnRequest,
            AskForApproval::Granular {
                sandbox_approval,
                rules,
                skill_approval,
                request_permissions,
                mcp_elicitations,
            } => CoreAskForApproval::Granular(CoreGranularApprovalConfig {
                sandbox_approval,
                rules,
                skill_approval,
                request_permissions,
                mcp_elicitations,
            }),
            AskForApproval::Never => CoreAskForApproval::Never,
        }
    }
}

impl From<CoreAskForApproval> for AskForApproval {
    fn from(value: CoreAskForApproval) -> Self {
        match value {
            CoreAskForApproval::UnlessTrusted => AskForApproval::UnlessTrusted,
            CoreAskForApproval::OnFailure => AskForApproval::OnFailure,
            CoreAskForApproval::OnRequest => AskForApproval::OnRequest,
            CoreAskForApproval::Granular(granular_config) => AskForApproval::Granular {
                sandbox_approval: granular_config.sandbox_approval,
                rules: granular_config.rules,
                skill_approval: granular_config.skill_approval,
                request_permissions: granular_config.request_permissions,
                mcp_elicitations: granular_config.mcp_elicitations,
            },
            CoreAskForApproval::Never => AskForApproval::Never,
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, TS)]
#[ts(type = r#""user" | "auto_review""#, export_to = "v2/")]
/// Configures who approval requests are routed to for review. Examples
/// include sandbox escapes, blocked network access, MCP approval prompts, and
/// ARC escalations. Defaults to `user`. `auto_review` uses a carefully
/// prompted subagent to gather relevant context and apply a risk-based
/// decision framework before approving or denying the request.
pub enum ApprovalsReviewer {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "auto_review")]
    AutoReview,
}

impl<'de> Deserialize<'de> for ApprovalsReviewer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "user" => Ok(Self::User),
            "auto_review" | "guardian_subagent" => Ok(Self::AutoReview),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["user", "auto_review"],
            )),
        }
    }
}

impl JsonSchema for ApprovalsReviewer {
    fn schema_name() -> String {
        "ApprovalsReviewer".to_string()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        string_enum_schema_with_description(
            &["user", "auto_review"],
            "Configures who approval requests are routed to for review. Examples include sandbox escapes, blocked network access, MCP approval prompts, and ARC escalations. Defaults to `user`. `auto_review` uses a carefully prompted subagent to gather relevant context and apply a risk-based decision framework before approving or denying the request. The legacy value `guardian_subagent` is accepted for compatibility.",
        )
    }
}

fn string_enum_schema_with_description(values: &[&str], description: &str) -> Schema {
    let mut schema = SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        metadata: Some(Box::new(Metadata {
            description: Some(description.to_string()),
            ..Default::default()
        })),
        ..Default::default()
    };
    schema.enum_values = Some(
        values
            .iter()
            .map(|value| JsonValue::String((*value).to_string()))
            .collect(),
    );
    Schema::Object(schema)
}

impl ApprovalsReviewer {
    pub fn to_core(self) -> CoreApprovalsReviewer {
        match self {
            ApprovalsReviewer::User => CoreApprovalsReviewer::User,
            ApprovalsReviewer::AutoReview => CoreApprovalsReviewer::AutoReview,
        }
    }
}

impl From<CoreApprovalsReviewer> for ApprovalsReviewer {
    fn from(value: CoreApprovalsReviewer) -> Self {
        match value {
            CoreApprovalsReviewer::User => ApprovalsReviewer::User,
            CoreApprovalsReviewer::AutoReview => ApprovalsReviewer::AutoReview,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename_all = "kebab-case", export_to = "v2/")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub fn to_core(self) -> CoreSandboxMode {
        match self {
            SandboxMode::ReadOnly => CoreSandboxMode::ReadOnly,
            SandboxMode::WorkspaceWrite => CoreSandboxMode::WorkspaceWrite,
            SandboxMode::DangerFullAccess => CoreSandboxMode::DangerFullAccess,
        }
    }
}

impl From<CoreSandboxMode> for SandboxMode {
    fn from(value: CoreSandboxMode) -> Self {
        match value {
            CoreSandboxMode::ReadOnly => SandboxMode::ReadOnly,
            CoreSandboxMode::WorkspaceWrite => SandboxMode::WorkspaceWrite,
            CoreSandboxMode::DangerFullAccess => SandboxMode::DangerFullAccess,
        }
    }
}
