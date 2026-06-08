//! Wire-compatible legacy tier names retained until runtime-core lands a Sprite-native plan abstraction.

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlanType {
    Known(KnownTier),
    Unknown(String),
}

impl PlanType {
    pub fn from_raw_value(raw: &str) -> Self {
        match raw.to_ascii_lowercase().as_str() {
            "free" => Self::Known(KnownTier::Free),
            "go" | "individual" => Self::Known(KnownTier::Individual),
            "plus" | "advanced" | "premium" => Self::Known(KnownTier::Premium),
            "pro" | "professional" => Self::Known(KnownTier::Professional),
            "prolite" | "professional_lite" | "professional_essentials" => {
                Self::Known(KnownTier::ProfessionalEssentials)
            }
            "team" => Self::Known(KnownTier::Team),
            "self_serve_business_usage_based" | "business_usage_based" | "business_metered" => {
                Self::Known(KnownTier::BusinessMetered)
            }
            "business" => Self::Known(KnownTier::Business),
            "enterprise_cbp_usage_based" | "enterprise_usage_based" | "enterprise_metered" => {
                Self::Known(KnownTier::EnterpriseMetered)
            }
            "enterprise" | "hc" => Self::Known(KnownTier::Enterprise),
            "education" | "edu" => Self::Known(KnownTier::Education),
            _ => Self::Unknown(raw.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnownTier {
    Free,
    #[serde(alias = "go")]
    Individual,
    #[serde(rename = "premium", alias = "advanced", alias = "plus")]
    Premium,
    #[serde(alias = "pro")]
    Professional,
    #[serde(
        rename = "professional_essentials",
        alias = "professional_lite",
        alias = "prolite"
    )]
    ProfessionalEssentials,
    Team,
    #[serde(
        rename = "business_metered",
        alias = "business_usage_based",
        alias = "self_serve_business_usage_based"
    )]
    BusinessMetered,
    Business,
    #[serde(
        rename = "enterprise_metered",
        alias = "enterprise_usage_based",
        alias = "enterprise_cbp_usage_based"
    )]
    EnterpriseMetered,
    #[serde(alias = "hc")]
    Enterprise,
    #[serde(alias = "education")]
    Education,
}

impl KnownTier {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Individual => "Individual",
            Self::Premium => "Premium",
            Self::Professional => "Professional",
            Self::ProfessionalEssentials => "Professional Essentials",
            Self::Team => "Team",
            Self::BusinessMetered => "Business Metered",
            Self::Business => "Business",
            Self::EnterpriseMetered => "Enterprise Metered",
            Self::Enterprise => "Enterprise",
            Self::Education => "Education",
        }
    }

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Individual => "individual",
            Self::Premium => "premium",
            Self::Professional => "professional",
            Self::ProfessionalEssentials => "professional_essentials",
            Self::Team => "team",
            Self::BusinessMetered => "business_metered",
            Self::Business => "business",
            Self::EnterpriseMetered => "enterprise_metered",
            Self::Enterprise => "enterprise",
            Self::Education => "education",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct RefreshTokenFailedError {
    pub reason: RefreshTokenFailedReason,
    pub message: String,
}

impl RefreshTokenFailedError {
    pub fn new(reason: RefreshTokenFailedReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshTokenFailedReason {
    Expired,
    Exhausted,
    Revoked,
    Other,
}

#[cfg(test)]
mod tests {
    use super::KnownTier;
    use super::PlanType;
    use pretty_assertions::assert_eq;

    #[test]
    fn plan_type_deserializes_raw_aliases() {
        assert_eq!(
            serde_json::from_str::<PlanType>("\"hc\"").expect("hc should deserialize"),
            PlanType::Known(KnownTier::Enterprise)
        );
        assert_eq!(
            serde_json::from_str::<PlanType>("\"education\"")
                .expect("education should deserialize"),
            PlanType::Known(KnownTier::Education)
        );
        assert_eq!(
            serde_json::from_str::<PlanType>("\"plus\"").expect("plus should deserialize"),
            PlanType::Known(KnownTier::Premium)
        );
        assert_eq!(
            serde_json::from_str::<PlanType>("\"professional_lite\"")
                .expect("professional_lite should deserialize"),
            PlanType::Known(KnownTier::ProfessionalEssentials)
        );
    }
}
