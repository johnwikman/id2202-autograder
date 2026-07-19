# CDN cache

Third-party web assets (Bootstrap) served locally instead of from a public CDN,
so the web UI and generated docs work without internet access.

`bom.toml` lists each asset's download source, expected `sha384`, and served
path. `build.rs` downloads any missing/stale file, verifies its hash, and
embeds it into the binaries. Commit downloaded files when they change.

Anything in this directory except for `bom.toml` and `README.md` are
automatically generated/downloaded, and should be considered read-only.
