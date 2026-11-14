# Public NEAR demonstration

This deployment serves `https://gnorix.online` through nginx. The Rust app and
its private NEAR sandbox listen on loopback. Every visitor shares the same two
test wallets, balances and active approval. Restarting the service creates a new
chain, enrollments and balances; no mainnet or public-testnet account is used.

The service keeps public enrollment material and the current attempt in memory,
without writing signature archives or per-attempt metadata files. The sandbox
itself retains submitted transactions in its chain history.

The backend runs as the dedicated `gnorix` system user. `gnorix-near-demo.service`
provides automatic restart, protected state storage and a 1,200 MiB memory ceiling.
The host needs Linux x86_64, systemd, nginx, CA certificates and the standard
Ubuntu 24.04 C/C++ runtime libraries. Rust, Cargo, Docker and a separate RPC
service are not needed on the deployment host.

## Prepare a release

On a trusted Linux x86_64 build machine, use the repository's pinned toolchain.
Python 3.11 or newer is needed only to prepare the bundle:

```sh
cargo build -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked

RUSTFLAGS='-C link-arg=-s --cfg near' cargo build --locked --release \
  --target wasm32-unknown-unknown --lib --no-default-features \
  --manifest-path examples/near/Cargo.toml --target-dir target/near-demo/contracts

RUSTFLAGS='-C link-arg=-s --cfg near' cargo build --locked --release \
  --target wasm32-unknown-unknown --lib --no-default-features \
  --manifest-path examples/near/token/Cargo.toml --target-dir target/near-demo/contracts
```

The Wasm target must be installed for that toolchain. Obtain the official native
NEAR sandbox **2.14.1** binary; starting the local demo once installs it into
Cargo's build cache. Its ELF interpreter must be `/lib64/ld-linux-x86-64.so.2`.

```sh
python3 examples/near/deploy/package.py \
  --binary target/release/gnorix-near-demo \
  --sandbox /path/to/near-sandbox \
  --contracts target/near-demo/contracts/wasm32-unknown-unknown/release \
  --output target/near-demo-release

tar -C target/near-demo-release -czf target/near-demo-release.tar.gz .
sha256sum target/near-demo-release.tar.gz
```

The output directory must be absent or empty. The packager copies the app's
exact dynamic loader and resolved libraries, which also supports a Nix build
host. The launcher uses that loader for the app only; it does not export
`LD_LIBRARY_PATH` into the sandbox's environment. The official sandbox uses the
deployment host's normal runtime libraries.

The release includes both Wasm contracts, the sandbox binary and `SHA256SUMS`.
`run` sets `GNORIX_DEMO_CONTRACTS_DIR` and `NEAR_SANDBOX_BIN_PATH`, so startup
requires no source checkout, compiler or artifact download. The package command
accepts trusted build artifacts only; `ldd` and the sandbox version check execute
local programs while preparing the release.

## Install

Back up the existing nginx configuration before making changes. Preserve other
sites, listeners and services. Transfer the release archive, the two deployment
configuration files and the supplied TLS pair over an authenticated SSH channel.
Verify the archive's SHA-256 against the build machine before extraction.

Create the `gnorix` system user with home `/var/lib/gnorix-near-demo` and no login
shell. Extract each release as root into a new directory under
`/opt/gnorix-near-demo/releases/`, then run `sha256sum -c SHA256SUMS` there.
Keep release files root-owned and readable by the service user. Point
`/opt/gnorix-near-demo/current` at the selected release; use an atomic symlink
replacement for subsequent updates and keep the preceding release for rollback.

Install the service file as
`/etc/systemd/system/gnorix-near-demo.service`. Install `nginx.conf` as
`/etc/nginx/sites-available/gnorix.online` and enable its symlink under
`sites-enabled`. The DNS A record must point to this host. The supplied
certificate covers `gnorix.online` only, without `www`.

Place the matching TLS files outside all release and web directories:

- `/etc/nginx/tls/gnorix.online/fullchain.pem`
- `/etc/nginx/tls/gnorix.online/privkey.pem`

Keep their directory root-owned with mode `0700` and the private key mode `0600`.
Verify that the certificate and key have the same public key before activation.
Never put the private key or SSH credentials in Git or a release archive.

```sh
sudo systemd-analyze verify /etc/systemd/system/gnorix-near-demo.service
sudo systemctl daemon-reload
sudo systemctl enable --now gnorix-near-demo
sudo nginx -t
sudo systemctl reload nginx
```

The supplied TLS certificate is managed manually; these files do not configure
automatic renewal. Replace the matching TLS pair before expiry, validate nginx
and reload it. The HTTP virtual host reserves the standard ACME challenge path
for a future renewal setup.

## Verify and operate

Check `https://gnorix.online/api/state` until `phase` is `ready` and both wallets
are present. HTTP starts before initialization completes; status 200 or an active
systemd unit alone does not establish readiness. Inspect state without logging
the session token. The token protects mutations from cross-site requests; it is
public demo state, not visitor authentication.

Check normal TLS validation, HTTP-to-HTTPS redirection, static asset hashes and
a browser transfer in each direction. Confirm the receipt, Merkle path, nonce
and actual balances. Failed random captures must remain visible as failed
attempts, without silently replacing them with another sample. Also verify that
previously hosted sites still work and that app/RPC/P2P ports remain loopback-only.

```sh
sudo systemctl status gnorix-near-demo
sudo journalctl -u gnorix-near-demo -n 100 --no-pager
sudo systemctl restart gnorix-near-demo
```

A restart discards the shared demo session. Schedule updates after any active
transfer resolves. To roll back, atomically restore the previous `current`
symlink and restart the service. For the first deployment, rollback removes only
the Gnorix virtual-host symlink and stops its service; validate and reload nginx
while preserving existing sites.
