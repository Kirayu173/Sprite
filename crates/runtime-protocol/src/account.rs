//! Wire-compatible legacy tier names retained until runtime-core lands a Sprite-native plan abstraction.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use crate::auth::KnownTier;
use crate::auth::PlanType as AuthPlanType;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, JsonSchema, TS, Default)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase")]
pub enum PlanType {
    #[default]
    Free,
    Individual,
    Premium,
    Professional,
    #[serde(rename = "professional_essentials", alias = "professional_lite")]
    #[ts(rename = "professional_essentials")]
    ProfessionalEssentials,
    Team,
    #[serde(rename = "business_metered", alias = "business_usage_based")]
    #[ts(rename = "business_metered")]
    BusinessMetered,
    Business,
    #[serde(rename = "enterprise_metered", alias = "enterprise_usage_based")]
    #[ts(rename = "enterprise_metered")]
    EnterpriseMetered,
    Enterprise,
    Education,
    #[serde(other)]
    Unknown,
}

/// Account state returned by a model provider before it is adapted to an app-facing wire type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAccount {
    ApiKey,
    Managed { email: String, plan_type: PlanType },
    External { provider_name: String },
}

impl From<AuthPlanType> for PlanType {
    fn from(plan_type: AuthPlanType) -> Self {
        match plan_type {
            AuthPlanType::Known(plan) => plan.into(),
            AuthPlanType::Unknown(_) => Self::Unknown,
        }
    }
}

impl From<KnownTier> for PlanType {
    fn from(plan: KnownTier) -> Self {
        match plan {
            KnownTier::Free => Self::Free,
            KnownTier::Individual => Self::Individual,
            KnownTier::Premium => Self::Premium,
            KnownTier::Professional => Self::Professional,
            KnownTier::ProfessionalEssentials => Self::ProfessionalEssentials,
            KnownTier::Team => Self::Team,
            KnownTier::BusinessMetered => Self::BusinessMetered,
            KnownTier::Business => Self::Business,
            KnownTier::EnterpriseMetered => Self::EnterpriseMetered,
            KnownTier::Enterprise => Self::Enterprise,
            KnownTier::Education => Self::Education,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlanType;
    use crate::auth::KnownTier;
    use crate::auth::PlanType as AuthPlanType;
    use pretty_assertions::assert_eq;

    #[test]
    fn usage_based_plan_types_use_expected_wire_names() {
        assert_eq!(
            serde_json::to_string(&PlanType::BusinessMetered)
                .expect("business metered should serialize"),
            "\"business_metered\""
        );
        assert_eq!(
            serde_json::to_string(&PlanType::EnterpriseMetered)
                .expect("enterprise metered should serialize"),
            "\"enterprise_metered\""
        );
        assert_eq!(
            serde_json::to_string(&PlanType::ProfessionalEssentials)
                .expect("professional_essentials should serialize"),
            "\"professional_essentials\""
        );
        assert_eq!(
            serde_json::from_str::<PlanType>("\"business_usage_based\"")
                .expect("business usage based should deserialize"),
            PlanType::BusinessMetered
        );
        assert_eq!(
            serde_json::from_str::<PlanType>("\"professional_lite\"")
                .expect("professional_lite should deserialize"),
            PlanType::ProfessionalEssentials
        );
        assert_eq!(
            serde_json::from_str::<PlanType>("\"enterprise_usage_based\"")
                .expect("enterprise usage based should deserialize"),
            PlanType::EnterpriseMetered
        );
    }

    #[test]
    fn auth_plan_type_converts_to_account_plan_type() {
        assert_eq!(
            PlanType::from(AuthPlanType::Known(KnownTier::EnterpriseMetered)),
            PlanType::EnterpriseMetered
        );
        assert_eq!(
            PlanType::from(AuthPlanType::Known(KnownTier::Enterprise)),
            PlanType::Enterprise
        );
        assert_eq!(
            PlanType::from(AuthPlanType::Unknown("mystery-tier".to_string())),
            PlanType::Unknown
        );
    }
}
