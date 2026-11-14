# Public NEAR demonstration

This deployment serves `https://gnorix.online` through nginx. The Rust app and
its private NEAR sandbox listen on loopback. Every browser session receives its own two
test wallets, token contract, balances, Merkle enrollments and active approval.
All sessions use one sandbox process. Restarting the service creates a new
chain and invalidates all sessions; no mainnet or public-testnet account is used.

Sessions have an absolute **3,600-second lifetime**; polling does not renew it.
Only GET `/api/state` creates or resumes a session. Access ends at expiry;
teardown cancels work and releases its enrollment/attempt data, with blocking
computations releasing references at their next cancellation checkpoint.
Retirement revokes enrollments and resolves accepted transfers before deletion,
with a **45-second limit**; unresolved cleanup leaves accounts on the private
chain and logs the failure. Service shutdown gives concurrent session cleanup
a shared **15-second grace**. Neither expiry nor account deletion reverses a
submitted transaction or erases its chain history.

The registry admits at most **16 sessions** and one CPU-heavy job at a time;
capacity returns 503 with `Retry-After: 60`, without evicting an existing visitor.
Cookies are host-only, scoped to `/api`, `HttpOnly`, `SameSite=Strict`, and
`Secure` on the configured HTTPS origin. Writes additionally require that
session's separate CSRF token. Do not strip cookies or cache `/api/` responses
in nginx. Full lifecycle behavior is in the [demo README](../README.md#browser-sessions-and-data-lifetime).

The supplied service enrolls and searches combinations of **exactly five points**,
matching the local five-point experiment. It requires at least three anchors,
each separated from every other anchor by at least **6R**, and a minimum feature
priority of **0.1**. Other points must lie inside the anchor outline. R is the
measured average dark-ridge thickness. The service sets
`--points-per-combination 5`; omitting that flag in a standalone local launch
retains the example's default 3–6 point range.

The page marks the prototype's security limits and provides a short **How it
works** explanation. The service keeps public enrollment material and the
current attempt in memory per browser session, without writing signature archives or per-attempt
metadata files. The sandbox itself retains submitted transactions in its chain
history. Transfers attach 100 Tgas; the UI shows this limit separately from
measured gas consumption. Browser isolation is not biometric authentication;
the [descriptor limitation](../../../docs/near-sdk.md#biometric-security-boundary)
also applies to the hosted service.

The backend runs as the dedicated `gnorix` system user. `gnorix-near-demo.service`
provides automatic restart, protected state storage and a 1,200 MiB memory ceiling.
The host needs Linux x86_64, systemd, nginx, CA certificates and the standard
Ubuntu 24.04 C/C++ runtime libraries. Rust, Cargo, Docker and a separate RPC
service are not needed on the deployment host.

## Prepare a release

On a trusted Linux x86_64 build machine, use the repository's pinned toolchain.
Python 3.11 or newer is needed only to prepare the bundle:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
export PATH="$(dirname "$(rustup which --toolchain 1.99.1 rustc)"):${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
cargo build -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked

RUSTFLAGS='-C link-arg=-s --cfg near' cargo build --locked --release \
  --target wasm32-unknown-unknown --lib --no-default-features \
  --manifest-path examples/near/Cargo.toml --target-dir target/near-demo/contracts

RUSTFLAGS='-C link-arg=-s --cfg near' cargo build --locked --release \
  --target wasm32-unknown-unknown --lib --no-default-features \
  --manifest-path examples/near/token/Cargo.toml --target-dir target/near-demo/contracts
```

Use all tools from the selected toolchain; a system `cargo-clippy` from another
release is not interchangeable with the pinned compiler. See the
[development setup](../../../docs/development.md#workspace-and-setup).
Obtain the official native
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

Open the browser and wait for its `/api/state` to report `phase: ready` with both
wallets. Automated readiness checks must retain the same cookie; each cookieless
GET creates a session. Use `HEAD /` for liveness checks that must not allocate
wallets:

```sh
curl --fail --head https://gnorix.online/
```

HTTP starts before initialization completes; status 200 or an active
systemd unit alone does not establish readiness. Keep the session cookie and
returned CSRF token out of logs. They scope this public demonstration, not
authenticated access to a real user's wallet.

Check normal TLS validation, HTTP-to-HTTPS redirection, static asset hashes and
a browser transfer in each direction. Confirm the receipt, Merkle path, nonce
and actual balances. Failed random captures must remain visible as failed
attempts, without silently replacing them with another sample. Check two browser
profiles have distinct contracts and roots; an action in one must leave the
other's balances, nonce and capture unchanged. Cross-session CSRF requests must
return 403 and another visitor's capture ID 404. Expired writes/captures return
410; a fresh GET `/api/state` renews the session rather than reviving its keys.
Also verify that
previously hosted sites still work and that app/RPC/P2P ports remain loopback-only.

```sh
sudo systemctl status gnorix-near-demo
sudo journalctl -u gnorix-near-demo -n 100 --no-pager
sudo systemctl restart gnorix-near-demo
```

A restart discards all demo sessions. Schedule updates after active
transfers resolve. To roll back, restore the previous `current` symlink and its
matching service configuration, run `systemctl daemon-reload`, then restart the
service. Retain both when changing release arguments. For the first deployment, rollback removes only
the Gnorix virtual-host symlink and stops its service; validate and reload nginx
while preserving existing sites.
