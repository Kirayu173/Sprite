# Phase 1 Migration Review Resolution - 2026-06-09

Source review: `phase1-migration-review-2026-06-09.md`.

## Resolution Summary

All four findings were verified against the migration requirements and addressed.

1. `diagnostics` local and JSON log initialization was a valid finding.
   - Added `LocalTracingConfig`, `LocalLogFormat`, `local_env_filter`, and `init_local_tracing`.
   - The initializer installs `tracing_subscriber::fmt` with env filtering and supports text or JSON stderr output.
   - OTLP remains optional and is not required for local logs.

2. Plugin protocol official service residue was a valid finding.
   - Replaced `PluginAuthPolicy` and `authPolicy` with product-neutral `PluginSetupPolicy` and `setupPolicy`.
   - Removed the plugin-service `"ENABLED"` availability alias from the public protocol.
   - Updated comments so catalogs are described as local/generated/in-memory instead of remote-service backed.

3. Missing committed schema fixture verification was a valid finding.
   - Added `crates/app-protocol/schema-manifest.txt` as a committed generated-schema file list.
   - Added a regression test that regenerates schema fixtures and compares the generated file set to the committed manifest without committing hundreds of generated `.json` and `.ts` files.

4. Product-specific protocol names were a valid finding for public protocol naming.
   - Replaced model verification and turn moderation protocol names with provider-policy naming:
     `ProviderPolicyCheck`, `ProviderPolicyMetadata`, and provider-policy notification methods.
   - Replaced cyber-specific enum values with `ProviderPolicy` and `AdditionalReview`.
   - Historical serde aliases are retained only for replay/deserialization compatibility; generated schema and TypeScript fixtures expose the Sprite-owned names.

## Misjudgment Notes

No review item was dismissed as a full misjudgment.

The only compatibility exception is deliberate: runtime deserialization accepts legacy event and enum names through serde aliases so existing rollout/history data can still be read. These aliases are not exported in the committed app protocol schema or TypeScript fixtures, and they do not reintroduce official auth, cloud, marketplace, telemetry, or remote-control dependencies.
