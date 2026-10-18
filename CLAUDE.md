# Working on Grove

## Every change ships to production

A finished change isn't done until it's released. After committing work
on `main` (tests, `cargo fmt` and clippy clean):

1. **Release it:** `scripts/release.sh <next version>`. This pushes `main`
   and the tag, waits for the release workflow to publish the binaries,
   and updates the Homebrew formula in the tap. Bump the patch version
   (0.1.1 → 0.1.2) unless asked for a bigger bump. If only the formula
   step fails, re-run with `--formula-only <version>`.
2. **Install it here:** `cargo install --path .`, then press `R` in the
   dashboard.
3. **Update the fincas:** `scripts/deploy-fincas.sh`.

Report the released version, and each finca's version from step 3.
