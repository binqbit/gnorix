# Fingerprint Lab

Complete the one-time [local setup](local-testing.md), then run the web interface from the checkout root:

```sh
cargo run -p gnorix-lab --release --locked
```

Wait for the startup URL, then open **http://127.0.0.1:8765**. The first build
can take several minutes. This Rust application embeds its interface and
default image; it needs no frontend installation, dataset or external service. Append
`-- --port 8766` to choose another port; stop with Ctrl+C. The
[application README](../examples/fingerprint-lab/README.md) covers setup, optional
gallery paths and tests.

The Lab is a **single local testing workspace**: tabs and browser profiles share
its enrollment and current operation. It does not use the NEAR demo's per-browser
cookies or one-hour sessions. Its adjustable defaults and artifact generator
are independent of the [NEAR transaction demo](../examples/near/README.md).

## Test a signature

For the first run, keep the default image and algorithm settings. Use the full
image as a control: in **Sign with a fragment**, set **Artifact severity** to
**0** and move **Selected fragment radius** to its maximum. The circle then
covers the complete image. **Center circle** only restores the default smaller
circle; it does not select the whole image.

1. In **Enrollment**, adjust the parameters and select **Prepare features**. Inspect the actual detected points and algorithm regions. Region buttons highlight all eligible points inside each effective circle. Preparation checks the baseline candidate count against the enrollment memory budget.
2. Select **Create Merkle set**. The stand derives one key from each selected subset's rounded baseline, creates the package, checks its readback and displays its Merkle root and leaf count. It does not expand positional uncertainty during enrollment.
3. Switch to **Sign with a fragment**. Drag the circular selection or change its radius and center. Pixels outside the circle are shaded in the interface. Adjust **Artifact severity** to test spatially varying capture defects, or leave it at zero for the original pixels. Select **Prepare features** to analyze the circular capture and inspect its regions and points.
4. Select **Sign message**. For each ranked subset, the stand checks its baseline and then its positional uncertainty variants one at a time. It stops at the first enrolled member. On success, white rings identify every feature in the matching combination. The result shows the Ed25519 public key, signature, Merkle proof and full public authorization JSON.
5. **Verify signature** checks the authorization against the retained enrollment and current message. After a successful control, reduce or move the circle and repeat **Prepare features → Sign message**. Each search consumes its preparation, so prepare again before retrying or signing a new message.

For a negative control, change the **Message** field after a successful signature,
then select **Verify signature**. The existing signature must fail verification
for the changed message. This checks message binding; selecting **Sign message**
would instead create a new signature.

The circle used to select source pixels and the algorithm's search circles are different controls. The former is measured in image pixels; the latter use **R**, the freshly measured mean dark-ridge thickness. Circular extraction preserves the selected scan's grayscale values and supplies an explicit validity mask; the interface's shade is never an input to extraction. Overlay coordinates are translated back to the original image.

Every selected combination satisfies the configured [spatial policy](feature-combinations.md#spatial-policy): at least k pairwise-separated anchor points, with remaining points inside or on their convex hull. The initial values are **k = 3** and **distance = 6R**. Fragments reuse this distance with their own measured R. There is no separate diameter or fraction of image width. Checks happen before grid normalization; admitted combinations retain their full positional uncertainty search. Every eligible regional point must occur in a valid enrollment subset; otherwise the whole preparation fails with its identity and region.

Feature markers use the same style as the static fingerprint reports: red → orange → yellow → green on the fixed 0–1 priority scale, with opacity `0.4 + 0.5 × sqrt(priority)` (40–90%). Endings are circles, bifurcations are triangles, junctions are diamonds, isolated nodes are squares, and isolated cycles are plus signs. The marker size stays fixed for each kind. A small neutral underline identifies the selected region; a concentric white ring identifies a matching feature. These separate indicators preserve the marker’s priority color and opacity.

The viewer shows every feature that meets the shared **Minimum feature priority** setting, which defaults to **0.5** for both enrollment and signing. Drag its **0–1 slider** in steps of **0.01**, or enter an exact value in the linked number field. Changing the threshold invalidates the current preparation; an existing enrollment requires a new Merkle set with that threshold. There is no additional display threshold. Matched features retain their original priority color and opacity, plus a concentric white ring. The counter reports all eligible features; the palette still uses the full 0–1 scale.

## Test capture artifacts

Enrollment always uses the clean source. In signing mode, **Artifact severity** controls spatially varying deformation across the image, including ridge bending, merged ridges, bright dropouts and broad blur. Random light or dark cut bands, scratches and streaks can interrupt or obscure the ridge pattern. Streak thickness varies from R/4 to R, and streaks may be sharp, blurred or noisy. The effects vary by location instead of applying one uniform distortion. Zero restores the original pixels exactly. Higher levels combine stronger defects; near 100, the scan is heavily distorted and much of its ridge structure can become difficult to distinguish. This is a severity control, not a measured percentage of damaged area, lost information or recognition error. Spatial dimensions and displacement use the clean enrollment's measured ridge width R.

A changed severity or **Randomize** produces a new time-seeded variation. The seed is retained in the test record rather than displayed in the controls. Each preview is generated from the original image, so changes never accumulate. The server applies these defects before selecting the circular capture, and feature extraction uses the exact pixels shown in the preview. Changing the preview invalidates old signing preparations and overlays while retaining the enrolled Merkle set.

**Test altered scan** prepares the currently displayed fragment, searches its candidate keys and verifies any resulting signature in one operation. The manual **Prepare features** → **Sign message** flow remains available to inspect points before searching. **Verify signature** checks an existing authorization against the message; it does not extract a new fingerprint.

Public test records include the artifact level, seed, clean ridge-width scale, generator version and grayscale-image SHA-256 for reproduction. Preview revisions bind preparations to a specific image; stale preparations are rejected.

## Choose an image

The default synthetic fingerprint is always available. Optionally, the **Image library** also offers five locally imported SOCOFing originals. Select a thumbnail to change the source. This clears the active enrollment, prepared features, signature, artifacts and analysis cache, then returns to Enrollment. Saved `.grx` files remain on disk; numerical controls stay at their current values. Marker sizes adapt to the display so smaller scanner images remain readable.

The curated gallery contains five distinct subjects: **226 / right middle**, **270 / right index**, **53 / right ring**, **560 / left little** and **590 / left middle**. These 241 × 298 originals show different ridge patterns and source contrast. They are selected demonstration examples, not a representative evaluation set or measured top-five biometric accuracy ranking.

Import the gallery from a local version-2 [SOCOFing download](fingerprint-testing.md#retrieve-a-pinned-copy):

```sh
uv run --project tools/fingerprint-reports --locked python \
  tools/fingerprint-gallery/import_socofing.py \
  --source /absolute/path/to/SOCOFing
```

The importer verifies the pinned source hashes and preserves grayscale pixels without resizing. It writes five PNGs into the ignored **`test-data/fingerprint-gallery/`** directory. Dataset images are local research inputs and are not distributed with the repository; the [dataset use terms and required citations](fingerprint-testing.md#provenance-and-use) apply. Start or restart the stand after importing. For another location, pass `--output /path/to/gallery` to the importer and `-- --gallery /path/to/gallery` to the stand. Missing gallery files leave the bundled image available; a present file with the wrong dimensions or pixel digest is rejected explicitly.

## Parameters and outcomes

Each number field is paired with a slider, including circle coordinates and computation limits. Sliders use the displayed setting's bounds and step; the priority slider moves in steps of 0.01 while its number field retains exact values. Moving a slider changes only that parameter. The controls share the existing validation and become unavailable during computation; changing them does not automatically launch analysis or generate keys.

The controls determine admission and computation, not cryptographic strength
in bits. Maximum points limits each combination; it does not discard other
eligible regional features.

| Control | Lab default |
| --- | --- |
| Points per combination | 3–6 |
| Minimum feature priority | 0.5 |
| Region radius | 30R |
| Minimum anchors / pairwise distance | 3 / 6R |
| Positional tolerance | 1 neighbor step |
| Enrollment memory estimate | 2 GiB, adjustable from 1 to 10 GiB |
| Search work limit | 1,000,000,000 |

The NEAR demo's priority 0.1 and deployed five-point profile do not override
these Lab defaults.

All eligible points in a region remain available. Sparse circles expand
until they include the configured maximum count, or all available points if
fewer exist. The shared [weighted selection](feature-combinations.md#automatic-weighted-selection)
uses absolute priority bands with 2:1:1 group weights, then adds coverage
witnesses and compact local subsets. It is bounded, not exhaustive. Signing
keeps the same band preference and finishes a subset's positional neighborhood
before moving to the next. Any eligible point unable to enter a geometrically
valid enrollment subset fails preparation; a feasibility-work limit reports
incomplete selection separately. No pool multiplier or per-region combination
budget is exposed.

**Enrollment memory budget** defaults to **2 GiB** and accepts **1–10 GiB** under Computation limits. Preparation uses a conservative peak-memory estimate with reserved headroom to derive consistent limits for baseline candidates, retained leaves and package size. It shows the estimated peak and supported candidate limit before key generation. If the complete requested baseline set exceeds these limits, preparation fails; it never silently truncates the set. This is an estimate, not an operating-system memory cap: actual process memory may vary.

Positional uncertainty variants are generated during signing and are not stored
as additional enrollment keys. The estimate accounts for an already retained
set when preparing its replacement.

The **Search work limit** defaults to **1,000,000,000** and counts preparation, subset enumeration, deduplication, spatial checks, geometry and positional traversal. Actual candidate-key checks remain visible statistics. Rejected subsets consume work without generating a key. The generic signing API receives the same value as its candidate-attempt safety cap; for this fingerprint stream, each yielded candidate already consumes work, so the cap does not independently bind. Budget exhaustion reports **incomplete**, never a completed no-match result. Work units are not milliseconds, and this limit allocates no candidates upfront.

Preparation reuses one compact analyzed capture when the source revision, exact circle and extraction settings are unchanged. Changing region geometry, priority cutoff or work limits rebuilds the selection/search without reprocessing the pixels. A different circle or altered-image revision requires fresh analysis. The cache holds features and their crop offset, not a full diagnostic report or additional image copies.

Changing enrollment parameters, including minimum feature priority, requires a newly prepared and registered set. The cutoff is part of the key profile, not a display setting. Changing only the enrollment memory budget invalidates an enrollment preparation, but does not change the key profile or invalidate an existing registered set. Changing the search-work limit requires preparing the fragment again. The interface hides outdated overlays and the server rejects stale preparation IDs. Operations run one at a time while the page reports their current stage.

| Result | Meaning and next step |
| --- | --- |
| **Key found / MATCHED** | A member key was found; inspect its public key, selected features and proof. Confirm **Signature and Merkle proof verified**. |
| **No match found / NO MATCH** | The configured regional search finished without an enrolled key. Restore the full clean image as a control, then try a larger fragment. Many visible points do not guarantee that an enrolled subset is present. |
| **Search incomplete / INCOMPLETE** | The work budget stopped the search. Increase **Search work limit** if it is below the allowed maximum, or try a larger/cleaner fragment; prepare again. This is not evidence of a completed mismatch. |
| **Experiment failed / ERROR**, or a preparation error | Read the displayed reason. Too few eligible points, impossible anchor geometry, stale settings and a memory estimate over budget require different changes. Parameter changes that affect enrollment require a new Merkle set. |

The root identifies an enrolled set. It is distinct from the Ed25519 public key of the member used for a signature. Signing searches only regional subsets, so spatially excluded combinations can be missed. The selection and anchor policy uses profile v10 and is retained inside container-v2 `.grx` packages; signing reads the saved policy. Recreate earlier test enrollments rather than mixing the interpretations.

## Local data

Successful enrollment writes `<root>.grx` and a public preparation record under **`reports/fingerprint-lab/`**. The package retains canonical public reconstruction settings, including the explicit point-count range, anchor distance in R and anchor count, bound to the context's profile ID. Signing writes its public result record there. These artifacts survive ordinary fingerprint-report refreshes. The active enrollment and preparations are held in server memory; after a server restart, create a new test enrollment. Private keys are transient and are not exported.

The stand binds to loopback and requires its process-local CSRF token for mutations. All clients of this local process share its state; it is not a public multi-user service. It accepts catalog image IDs and bounded numeric controls; the HTTP API does not accept filesystem paths. Each enrollment uses a new public context and salt, so its root can differ between runs. The test suite is HKDF-SHA256/Ed25519 with no independent secret. These demonstrations use a synthetic fixture and selected research scans; they do not qualify repeatability across independent scanner captures or biometric key entropy.

## Validation scope

Run these independent checks from the repository root; none starts a server or
requires SOCOFing:

```sh
# Extraction, geometry and ranking units, plus image-pipeline integration tests.
cargo test -p gnorix-core --release --locked

# Lab HTTP, state transitions, image/artifact and resource handling.
cargo test -p gnorix-lab --release --locked
```

The image-pipeline integration suite can also run alone with
`cargo test -p gnorix-core --test fingerprint --release --locked`. Its checked-in
image test exercises extraction, measurements, contours, feature scores and
regional combinations. Use release mode for image processing; debug mode is
considerably slower. These automated checks do not produce a PNG report; use
the [report command](../tools/fingerprint-reports/README.md#run) for that.

Use the full source as a control, then test smaller circles and altered scans.
Inspect eligible features, regions, attempted keys and the final status. A crop
may omit every enrolled subset despite showing many features. Test positive and
negative controls under the actual retained profile; historical v7/v8 leaf
counts and first-match positions do not describe current profile v10.

A successful signature proves that a generated key belongs to the retained set.
It does not establish biometric identity. The [canonical security-boundary
section](near-sdk.md#biometric-security-boundary) records the observed cross-subject
descriptor match and the remaining validation requirements.
