# Fingerprint test stand

Run the local web interface from the checkout root:

```sh
cargo run -p gnorix-lab --release --locked
```

The stand is an independent Cargo application at [`examples/fingerprint-lab/`](../examples/fingerprint-lab/README.md), with its own manifest, Rust modules and embedded web assets. It is a workspace member named `gnorix-lab`, depending on the local `gnorix` library and shared example adapters. The workspace supplies the lockfile and build directory. You can also run `cargo run --release --locked` from the application directory. Building requires the repository checkout, including the shared test fixture.

Open **http://127.0.0.1:8765**. The server and all biometric and cryptographic operations run in Rust. HTML, CSS, JavaScript, the logo and the existing test fingerprint are bundled with the executable; no frontend installation or external services are needed. The first build needs Cargo dependencies. Use `-- --port 8766` after the command to choose another local port. Stop the server with Ctrl+C.

## Choose an image

The **Image library** offers the default synthetic fingerprint and five locally imported SOCOFing originals. Select a thumbnail to change the source. This clears the active enrollment, prepared features, signature, artifacts and analysis cache, then returns to Enrollment. Saved `.grx` files remain on disk; numerical controls stay at their current values. Marker sizes adapt to the display so smaller scanner images remain readable.

The curated gallery contains five distinct subjects: **226 / right middle**, **270 / right index**, **53 / right ring**, **560 / left little** and **590 / left middle**. These 241 × 298 originals show different ridge patterns and source contrast. They are selected demonstration examples, not a representative evaluation set or measured top-five biometric accuracy ranking.

Import the gallery from a local version-2 [SOCOFing download](fingerprint-testing.md#retrieve-a-pinned-copy):

```sh
uv run --project tools/fingerprint-reports --locked python \
  tools/fingerprint-gallery/import_socofing.py \
  --source /absolute/path/to/SOCOFing
```

The importer verifies the pinned source hashes and preserves grayscale pixels without resizing. It writes five PNGs into the ignored **`test-data/fingerprint-gallery/`** directory. Dataset images are local research inputs and are not distributed with the repository; the [dataset use terms and required citations](fingerprint-testing.md#provenance-and-use) apply. Start or restart the stand after importing. For another location, pass `--output /path/to/gallery` to the importer and `-- --gallery /path/to/gallery` to the stand. Missing gallery files leave the bundled image available; a present file with the wrong dimensions or pixel digest is rejected explicitly.

## Test a signature

1. In **Enrollment**, adjust the parameters and select **Prepare features**. Inspect the actual detected points and algorithm regions. Region buttons highlight all eligible points inside each effective circle. Preparation checks the baseline candidate count against the enrollment memory budget.
2. Select **Create Merkle set**. The stand derives one key from each selected subset's rounded baseline, creates the package, checks its readback and displays its Merkle root and leaf count. It does not expand positional uncertainty during enrollment.
3. Switch to **Sign with a fragment**. Drag the circular selection or change its radius and center. Pixels outside the circle are shaded in the interface. Adjust **Artifact severity** to test spatially varying capture defects, or leave it at zero for the original pixels. Select **Prepare features** to analyze the circular capture and inspect its regions and points.
4. Select **Sign message**. For each ranked subset, the stand checks its baseline and then its positional uncertainty variants one at a time. It stops at the first enrolled member. On success, white rings identify every feature in the matching combination. The result shows the Ed25519 public key, signature, Merkle proof and full public authorization JSON.
5. **Verify signature** checks the authorization against the retained enrollment and current message. Changing the message makes verification fail. A consumed search needs another preparation before retrying.

The circle used to select source pixels and the algorithm's search circles are different controls. The former is measured in image pixels; the latter use **R**, the freshly measured mean dark-ridge thickness. Circular extraction preserves the selected scan's grayscale values and supplies an explicit validity mask; the interface's shade is never an input to extraction. Overlay coordinates are translated back to the original image.

Every selected combination satisfies the configured [spatial policy](feature-combinations.md#spatial-policy): at least k pairwise-separated anchor points, with remaining points inside or on their convex hull. The initial values are **k = 3** and **distance = 6R**. Fragments reuse this distance with their own measured R. There is no separate diameter or fraction of image width. Checks happen before grid normalization; admitted combinations retain their full positional uncertainty search. Every eligible regional point must occur in a valid enrollment subset; otherwise the whole preparation fails with its identity and region.

Feature markers use the same style as the static fingerprint reports: red → orange → yellow → green on the fixed 0–1 priority scale, with opacity `0.4 + 0.5 × sqrt(priority)` (40–90%). Endings are circles, bifurcations are triangles, junctions are diamonds, isolated nodes are squares, and isolated cycles are plus signs. The marker size stays fixed for each kind. A small neutral underline identifies the selected region; a concentric white ring identifies a matching feature. These separate indicators preserve the marker’s priority color and opacity.

The viewer shows every feature that meets the shared **Minimum feature priority** setting, which defaults to **0.5** for both enrollment and signing. Drag its **0–1 slider** in steps of **0.01**, or enter an exact value in the linked number field. Changing the threshold invalidates the current preparation; an existing enrollment requires a new Merkle set with that threshold. There is no additional display threshold. Matched features retain their original priority color and opacity, plus a concentric white ring. The counter reports all eligible features; the palette still uses the full 0–1 scale.

## Test capture artifacts

Enrollment always uses the clean source. In signing mode, **Artifact severity** controls spatially varying deformation across the image, including ridge bending, merged ridges, bright dropouts and broad blur. Random light or dark cut bands, scratches and streaks can interrupt or obscure the ridge pattern. Streak thickness varies from R/4 to R, and streaks may be sharp, blurred or noisy. The effects vary by location instead of applying one uniform distortion. Zero restores the original pixels exactly. Higher levels combine stronger defects; near 100, the scan is heavily distorted and much of its ridge structure can become difficult to distinguish. This is a severity control, not a measured percentage of damaged area, lost information or recognition error. Spatial dimensions and displacement use the clean enrollment's measured ridge width R.

A changed severity or **Randomize** produces a new time-seeded variation. The seed is retained in the test record rather than displayed in the controls. Each preview is generated from the original image, so changes never accumulate. The server applies these defects before selecting the circular capture, and feature extraction uses the exact pixels shown in the preview. Changing the preview invalidates old signing preparations and overlays while retaining the enrolled Merkle set.

**Test altered scan** prepares the currently displayed fragment, searches its candidate keys and verifies any resulting signature in one operation. The manual **Prepare features** → **Sign message** flow remains available to inspect points before searching. **Verify signature** checks an existing authorization against the message; it does not extract a new fingerprint.

Public test records include the artifact level, seed, clean ridge-width scale, generator version and grayscale-image SHA-256 for reproduction. Preview revisions bind preparations to a specific image; stale preparations are rejected.

## Parameters and outcomes

Each number field is paired with a slider, including circle coordinates and computation limits. Sliders use the displayed setting's bounds and step; the priority slider moves in steps of 0.01 while its number field retains exact values. Moving a slider changes only that parameter. The controls share the existing validation and become unavailable during computation; changing them does not automatically launch analysis or generate keys.

The stand exposes minimum and maximum points per combination, minimum feature priority, region radius in R, minimum anchor count, minimum anchor distance in R, grid uncertainty steps, enrollment memory budget and one search-work budget. These determine admission and computation, not cryptographic strength in bits. The initial point-count range is **3–6**. Maximum points limits each combination; it does not discard other eligible regional features.

All points meeting the priority cutoff inside a region remain eligible. Sparse circles expand automatically to reach the configured maximum, or all available points if fewer exist. The [absolute priority bands](feature-combinations.md#automatic-weighted-selection) use the strongest included point: H `> 0.9`, M `0.5..=0.9`, L `< 0.5`. With N eligible regional points, enrollment targets 3N distinct valid subsets in H, M, H, L order, skips exhausted bands, then adds coverage witnesses and compact local groups around each point. Signing uses the same cycle without the 3N cap; positional variants of one subset finish before the next subset starts. Shares count subsets, not key attempts or point occurrences. There is no pool multiplier or per-region combination control. Any point unable to enter a valid subset fails the whole generation. A work limit prevents unbounded feasibility searches and reports that selection could not be completed.

**Enrollment memory budget** defaults to **2 GiB** and accepts **1–10 GiB** under Computation limits. Preparation uses a conservative peak-memory estimate with reserved headroom to derive consistent limits for baseline candidates, retained leaves and package size. It shows the estimated peak and supported candidate limit before key generation. If the complete requested baseline set exceeds these limits, preparation fails; it never silently truncates the set. This is an estimate, not an operating-system memory cap: actual process memory may vary.

The estimate reserves 512 MiB for working data, then adds 512 bytes per planned baseline candidate and 96 bytes per leaf in the already retained enrollment. Without a previous set, the default budget permits 3,145,728 candidates; 10 GiB permits 19,922,944. Encoded package limits grow with that allowance, and temporary tree and byte-buffer copies are released before package readback. The signing uncertainty product is generated on demand and does not count as retained enrollment keys.

The **Search work limit** defaults to **1,000,000,000** and counts preparation, subset enumeration, deduplication, spatial checks, geometry and positional traversal. Actual candidate-key checks remain visible statistics. Rejected subsets consume work without generating a key. The generic signing API receives the same value as its candidate-attempt safety cap; for this fingerprint stream, each yielded candidate already consumes work, so the cap does not independently bind. Budget exhaustion reports **incomplete**, never a completed no-match result. Work units are not milliseconds, and this limit allocates no candidates upfront.

Preparation reuses one compact analyzed capture when the source revision, exact circle and extraction settings are unchanged. Changing region geometry, priority cutoff or work limits rebuilds the selection/search without reprocessing the pixels. A different circle or altered-image revision requires fresh analysis. The cache holds features and their crop offset, not a full diagnostic report or additional image copies.

Changing enrollment parameters, including minimum feature priority, requires a newly prepared and registered set. The cutoff is part of the key profile, not a display setting. Changing only the enrollment memory budget invalidates an enrollment preparation, but does not change the key profile or invalidate an existing registered set. Changing the search-work limit requires preparing the fragment again. The interface hides outdated overlays and the server rejects stale preparation IDs. Operations run one at a time while the page reports their current stage.

- **Matched:** a member key was found, and both signature and Merkle proof were verified.
- **No match:** the configured regional search was exhausted without a member.
- **Incomplete:** the work budget stopped signing. This is not a completed no-match result. Enrollment baseline counts that exceed the memory-derived limits are rejected during preparation, before a package is created.
- **Error:** invalid input, insufficient usable data or another failed operation; the page retains the error message.

The root identifies an enrolled set. It is distinct from the Ed25519 public key of the member used for a signature. Signing searches only regional subsets, so spatially excluded combinations can be missed. The selection and anchor policy uses profile v9 and is retained inside container-v2 `.grx` packages; signing reads the saved policy. Recreate earlier test enrollments rather than mixing the interpretations.

## Local data

Successful enrollment writes `<root>.grx` and a public preparation record under **`reports/fingerprint-lab/`**. The package retains canonical public reconstruction settings, including the explicit point-count range, anchor distance in R and anchor count, bound to the context's profile ID. Signing writes its public result record there. These artifacts survive ordinary fingerprint-report refreshes. The active enrollment and preparations are held in server memory; after a server restart, create a new test enrollment. Private keys are transient and are not exported.

The stand binds to loopback and requires its session token for mutations. It accepts catalog image IDs and bounded numeric controls; the HTTP API does not accept filesystem paths. Each enrollment uses a new public context and salt, so its root can differ between runs. The test suite is HKDF-SHA256/Ed25519 with no independent secret. These demonstrations use a synthetic fixture and selected research scans; they do not qualify repeatability across independent scanner captures or biometric key entropy.

## Validation scope

Use the full source as a control, then test smaller circles and altered scans. Inspect the actual eligible features, selected regions, attempted keys and final status; fewer visible points or a changed extraction can prevent a crop from containing an enrolled subset. Profile-v8 enrollments need fresh positive and negative controls; previous leaf counts and crop outcomes do not describe the current schedule.

A successful signature verifies that the generated key belongs to the retained Merkle set; this alone does not qualify biometric identity verification.

Historical profile-v7 full-source controls verified enrollment, reconstruction and signatures for the synthetic fixture and five gallery images. Those subset/leaf counts and first-match positions, and later profile-v8 figures, must not be treated as current profile-v9 results.

The historical profile-v7 negative control enrolling 270 / right index and signing with 226 / right middle found a member after 414 candidates; both signature and proof verified. Distinct point subsets can quantize to identical descriptors. The new band schedule does not change that encoding, so biometric discrimination still requires separate validation.
