# Pricing Resources

These JSON snapshots are vendored from [OpenUsage](https://github.com/robinebers/openusage/tree/main/Sources/OpenUsage/Resources) under its MIT license (Copyright 2026 Robin Ebers). DevClash retains the original model names, aliases and rates. The supplement records its own update date and its upstream sources; LiteLLM and models.dev snapshots retain their feed metadata.

The native pricing engine checks the same upstream feeds and uses these snapshots while offline. Costs are API-rate estimates unless a provider log supplies a recorded cost. Unknown prices keep their tokens and show incomplete cost coverage.

To update the bundled files, run OpenUsage's `script/update_pricing_snapshots.sh` in the upstream checkout, verify its supplement separately against the published provider rates, and copy the three resulting files into this directory. Keep the original MIT notice in `OPENUSAGE-LICENSE.txt`. No snapshot changes require an app version bump during development; publishing an installer does require owner approval.
