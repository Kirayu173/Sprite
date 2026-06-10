$original = Get-Content '_example/codex/codex-rs/core/src/config/permissions.rs'
$new = Get-Content 'crates/config/src/permissions.rs'
$diff = Compare-Object $original $new
$diff | Where-Object { $_.SideIndicator -eq '<=' } | Select-Object -First 200 | ForEach-Object { $_.InputObject }
"----"
$diff | Where-Object { $_.SideIndicator -eq '=>' } | Select-Object -First 200 | ForEach-Object { $_.InputObject }
