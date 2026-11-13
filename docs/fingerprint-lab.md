# Fingerprint test stand

Run the local web interface from the checkout root:

```sh
cargo run -p gnorix --example fingerprint_lab --release --locked
```

Open **http://127.0.0.1:8765**. The server and all biometric and cryptographic operations run in Rust. HTML, CSS, JavaScript, the logo and the existing test fingerprint are bundled with the executable; no frontend installation or external services are needed. The first build needs Cargo dependencies. Use `-- --port 8766` after the command to choose another local port. Stop the server with Ctrl+C.

## Test a signature

1. In **Enrollment**, adjust the parameters and select **Prepare features**. Inspect the actual detected points and algorithm regions. Region buttons highlight each selected pool. Preparation also checks the complete grid-variant count before enrollment.
2. Select **Create Merkle set**. The stand derives the enrolled keys, creates the package, checks its readback and displays its Merkle root and leaf count.
3. Switch to **Sign with a fragment**. Drag the circular selection or change its radius and center. Pixels outside the circle are shaded in the interface. Adjust **Artifact severity** to test spatially varying capture defects, or leave it at zero for the original pixels. Select **Prepare features** to extract the circular capture again and inspect its regions and points.
4. Select **Sign message**. The stand searches the prepared candidate stream and stops at the first enrolled member. On success, white rings identify the visible features in the matching combination. The result shows the Ed25519 public key, signature, Merkle proof and full public authorization JSON.
5. **Verify signature** checks the authorization against the retained enrollment and current message. Changing the message makes verification fail. A consumed search needs another preparation before retrying.

The circle used to select source pixels and the algorithm's search circles are different controls. The former is measured in image pixels; the latter use **R**, the freshly measured mean dark-ridge thickness. Circular extraction preserves the selected scan's grayscale values and supplies an explicit validity mask; the interface's shade is never an input to extraction. Overlay coordinates are translated back to the original image.

Feature markers use the same style as the static fingerprint reports: red → orange → yellow → green on the fixed 0–1 priority scale, with opacity `0.4 + 0.5 × sqrt(priority)` (40–90%). Endings are circles, bifurcations are triangles, junctions are diamonds, isolated nodes are squares, and isolated cycles are plus signs. The marker size stays fixed for each kind. A small neutral underline identifies the selected pool; a concentric white ring identifies a matching feature. These separate indicators preserve the marker’s priority color and opacity.

The viewer hides features with priority below **0.25**, including their pool and match indicators, and shows the visible/total feature count. This is a display filter only: all extracted features remain available to combination generation, signing and result records. The palette still uses the full 0–1 scale.

## Test capture artifacts

Enrollment always uses the clean source. In signing mode, **Artifact severity** controls spatially varying deformation across the image, including ridge bending, merged ridges, bright dropouts and broad blur. Random light or dark cut bands, scratches and streaks can interrupt or obscure the ridge pattern. Streak thickness varies from R/4 to R, and streaks may be sharp, blurred or noisy. The effects vary by location instead of applying one uniform distortion. Zero restores the original pixels exactly. Higher levels increase the strength of these defects; the value does not describe an affected-area percentage or an error probability. Spatial dimensions and displacement use the clean enrollment's measured ridge width R.

A changed severity or **Randomize** produces a new time-seeded variation. The seed is retained in the test record rather than displayed in the controls. Each preview is generated from the original image, so changes never accumulate. The server applies these defects before selecting the circular capture, and feature extraction uses the exact pixels shown in the preview. Changing the preview invalidates old signing preparations and overlays while retaining the enrolled Merkle set.

**Test altered scan** prepares the currently displayed fragment, searches its candidate keys and verifies any resulting signature in one operation. The manual **Prepare features** → **Sign message** flow remains available to inspect points before searching. **Verify signature** checks an existing authorization against the message; it does not extract a new fingerprint.

Public test records include the artifact level, seed, clean ridge-width scale, generator version and grayscale-image SHA-256 for reproduction. Preview revisions bind preparations to a specific image; stale preparations are rejected.

## Parameters and outcomes

The stand exposes minimum subset size, enrollment pool multiplier, search radius in R, selected combinations per region, grid uncertainty steps, attempt limit and search-work limit. It uses the same library mechanisms as the command-line examples. The weight curve remains `0.1 + 0.9 × priority²`.

The attempt limit defaults to **1,000,000,000 candidate keys** and accepts values from 1 through that maximum. Search remains lazy: this capacity does not allocate or generate a billion candidates upfront. The separate work budget counts preparation, subset enumeration, deduplication and geometry operations, so it can stop a search before the key-attempt limit is reached. Both limits retain the **incomplete** outcome; neither means that all possible candidates were tested.

Changing enrollment parameters requires a newly prepared and registered set. Changing the search-work limit requires preparing the fragment again; the attempt limit can change before signing. The interface hides outdated overlays and the server rejects stale preparation IDs. Operations run one at a time while the page reports their current stage.

- **Matched:** a member key was found, and both signature and Merkle proof were verified.
- **No match:** the configured regional search was exhausted without a member.
- **Incomplete:** an attempt or search-work limit stopped signing. This is not a completed no-match result. Excessive enrollment variant counts are rejected during preparation, before a package is created.
- **Error:** invalid input, insufficient usable data or another failed operation; the page retains the error message.

The root identifies an enrolled set. It is distinct from the Ed25519 public key of the member used for a signature. Signing searches only regional subsets, so spatially excluded combinations can be missed.

## Local data

Successful enrollment writes `<root>.grx` and a public preparation record under **`reports/fingerprint-lab/`**. Signing writes its public result record there. These artifacts survive ordinary fingerprint-report refreshes. The active enrollment and preparations are held in server memory; after a server restart, create a new test enrollment. Private keys are transient and are not exported.

The stand binds to loopback and requires its session token for mutations. It accepts the bundled fixture and bounded numeric controls; the HTTP API does not accept filesystem paths. Each enrollment uses a new public context and salt, so its root can differ between runs. The test suite is HKDF-SHA256/Ed25519 with no independent secret. This is a demonstration on a synthetic image, not a qualification of repeatability across independent scanner captures or biometric key entropy.

## Checked example

On the bundled fixture with the defaults, the full preparation produced four regions, 48 combinations and 111,132 grid assignments. A circular capture centered at `(543, 724)` with radius `369 px` produced 29 scored features in two search regions. Signing found a member after 51 candidates using features `11, 33, 38, 42`; signature and Merkle verification passed. A one-attempt search returned **incomplete** without an authorization. Changed message bytes failed verification.

The browser workflow was checked at desktop and 390px mobile widths. See the local [stand preview](../reports/fingerprint-lab/preview.png). These are observations on the synthetic fixture, not acceptance rates across captures.
