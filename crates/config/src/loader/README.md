# `config` loader

This module is the canonical place to load and describe Sprite configuration layers (system, user, project, thread, and CLI/session overrides) and to produce:

- An effective merged TOML config.
- Per-key origins metadata for the winning layer.
- Per-layer versions used for optimistic concurrency and conflict detection.

## Public surface

Exported from `config::loader`:

- `load_config_layers_state(fs, sprite_home, cwd_opt, cli_overrides, options, thread_config_loader) -> ConfigLayerStack`
- `ConfigLayerStack`
  - `effective_config() -> toml::Value`
  - `origins() -> HashMap<String, ConfigLayerMetadata>`
  - `layers_high_to_low() -> Vec<ConfigLayer>`
  - `with_user_config(user_config) -> ConfigLayerStack`
- `ConfigLayerEntry`, one layer's `{name, config, version, disabled_reason}`.
- `ConfigLoadOptions`, user-facing load behavior such as strict config validation.
- `LoaderOverrides`, test and explicit path overrides.
- `merge_toml_values(base, overlay)`, a public helper used elsewhere.

## Layering model

Precedence is top overrides bottom:

1. `SessionFlags`, CLI overrides applied as dotted-path TOML writes.
2. `Project` config, `.sprite/config.toml`.
3. `User` profile config, when present.
4. `User` config, `config.toml`.
5. `System` config, `/etc/sprite/config.toml` or the Windows system config path.

`ConfigLayerStack` stores layers in the opposite order internally: lowest precedence first, highest precedence last, so later layers override earlier layers when folded. Thread config entries supplied by `thread_config_loader` are inserted according to their translated `ConfigLayerSource` precedence.

Layers with a `disabled_reason` are still surfaced for UI, but are ignored when computing the effective config and origins metadata. This is what `ConfigLayerStack::effective_config()` implements.

## Typical usage

Most callers want the effective config plus metadata:

```rust
use config::LoaderOverrides;
use config::NoopThreadConfigLoader;
use config::loader::load_config_layers_state;
use utils_absolute_path::AbsolutePathBuf;
use toml::Value as TomlValue;

let cli_overrides: Vec<(String, TomlValue)> = Vec::new();
let sprite_home = AbsolutePathBuf::current_dir()?;
let cwd = AbsolutePathBuf::current_dir()?;
let layers = load_config_layers_state(
    fs,
    sprite_home.as_path(),
    Some(cwd),
    &cli_overrides,
    LoaderOverrides::default(),
    &NoopThreadConfigLoader,
).await?;

let effective = layers.effective_config();
let origins = layers.origins();
let layers_for_ui = layers.layers_high_to_low();
```

## Internal layout

Implementation is split by concern:

- `state.rs`: public types (`ConfigLayerEntry`, `ConfigLayerStack`) plus merge/origins convenience methods.
- `overrides.rs`: CLI dotted-path overrides to TOML session-flags layer.
- `merge.rs`: recursive TOML merge.
- `fingerprint.rs`: stable per-layer hashing and per-key origins traversal.
