# Fingerprint test stand

Run the local web interface from the checkout root:

```sh
cargo run -p gnorix --example fingerprint_lab --release --locked
```

Open **http://127.0.0.1:8765**. The server and all biometric and cryptographic operations run in Rust. HTML, CSS, JavaScript, the logo and the existing test fingerprint are bundled with the executable; no frontend installation or external services are needed. The first build needs Cargo dependencies. Use `-- --port 8766` after the command to choose another local port. Stop the server with Ctrl+C.

## Choose an image

The **Image library** offers the default synthetic fingerprint and five locally imported SOCOFing originals. Select a thumbnail to change the source. This clears the active enrollment, prepared features, signature, artifacts and analysis cache, then returns to Enrollment. Saved `.grx` files remain on disk; numerical controls stay at their current values. Marker sizes adapt to the display so smaller scanner images remain readable.

The curated gallery contains five distinct subjects: **226 / right middle**, **270 / right index**, **53 / right ring**, **560 / left little** and **590 / left middle**. These 241 × 298 originals show different ridge patterns and source contrast. All five pass preparation with the default priority and anchor rules. They are selected demonstration examples, not a representative evaluation set or measured top-five biometric accuracy ranking.

Import the gallery from a local version-2 [SOCOFing download](fingerprint-testing.md#retrieve-a-pinned-copy):

```sh
uv run --project tools/fingerprint-reports --locked python \
  tools/fingerprint-gallery/import_socofing.py \
  --source /absolute/path/to/SOCOFing
```

The importer verifies the pinned source hashes and preserves grayscale pixels without resizing. It writes five PNGs into the ignored **`test-data/fingerprint-gallery/`** directory. Dataset images are local research inputs and are not distributed with the repository; the [dataset use terms and required citations](fingerprint-testing.md#provenance-and-use) apply. Start or restart the stand after importing. For another location, pass `--output /path/to/gallery` to the importer and `-- --gallery /path/to/gallery` to the stand. Missing gallery files leave the bundled image available; a present file with the wrong dimensions or pixel digest is rejected explicitly.

## Test a signature

1. In **Enrollment**, adjust the parameters and select **Prepare features**. Inspect the actual detected points and algorithm regions. Region buttons highlight each selected pool. Preparation checks the baseline candidate count against the enrollment memory budget.
2. Select **Create Merkle set**. The stand derives one key from each selected subset's rounded baseline, creates the package, checks its readback and displays its Merkle root and leaf count. It does not expand positional uncertainty during enrollment.
3. Switch to **Sign with a fragment**. Drag the circular selection or change its radius and center. Pixels outside the circle are shaded in the interface. Adjust **Artifact severity** to test spatially varying capture defects, or leave it at zero for the original pixels. Select **Prepare features** to analyze the circular capture and inspect its regions and points.
4. Select **Sign message**. For each ranked subset, the stand checks its baseline and then its positional uncertainty variants one at a time. It stops at the first enrolled member. On success, white rings identify every feature in the matching combination. The result shows the Ed25519 public key, signature, Merkle proof and full public authorization JSON.
5. **Verify signature** checks the authorization against the retained enrollment and current message. Changing the message makes verification fail. A consumed search needs another preparation before retrying.

The circle used to select source pixels and the algorithm's search circles are different controls. The former is measured in image pixels; the latter use **R**, the freshly measured mean dark-ridge thickness. Circular extraction preserves the selected scan's grayscale values and supplies an explicit validity mask; the interface's shade is never an input to extraction. Overlay coordinates are translated back to the original image.

Every selected combination satisfies the configured [spatial policy](feature-combinations.md#spatial-policy): minimum diameter, at least k pairwise-separated anchor points, and all remaining points inside or on the anchors' convex hull. The initial values are diameter S/3, k = 3 and anchor separation S/6, where S is the enrolled bounds' shorter side. Enrollment stores both distances in R units; fragments reuse them with their own measured R instead of weakening them to fit a crop. Checks happen before grid normalization; admitted combinations retain their full positional uncertainty search. An enrollment pool point that cannot enter any valid subset causes the whole preparation to fail with a diagnostic.

Feature markers use the same style as the static fingerprint reports: red → orange → yellow → green on the fixed 0–1 priority scale, with opacity `0.4 + 0.5 × sqrt(priority)` (40–90%). Endings are circles, bifurcations are triangles, junctions are diamonds, isolated nodes are squares, and isolated cycles are plus signs. The marker size stays fixed for each kind. A small neutral underline identifies the selected pool; a concentric white ring identifies a matching feature. These separate indicators preserve the marker’s priority color and opacity.

The viewer shows every feature that meets the shared **Minimum feature priority** setting, which defaults to **0.5** for both enrollment and signing. Drag its **0–1 slider** in steps of **0.01**, or enter an exact value in the linked number field. Changing the threshold invalidates the current preparation; an existing enrollment requires a new Merkle set with that threshold. There is no additional display threshold. Matched features retain their original priority color and opacity, plus a concentric white ring. The counter reports all eligible features; the palette still uses the full 0–1 scale.

## Test capture artifacts

Enrollment always uses the clean source. In signing mode, **Artifact severity** controls spatially varying deformation across the image, including ridge bending, merged ridges, bright dropouts and broad blur. Random light or dark cut bands, scratches and streaks can interrupt or obscure the ridge pattern. Streak thickness varies from R/4 to R, and streaks may be sharp, blurred or noisy. The effects vary by location instead of applying one uniform distortion. Zero restores the original pixels exactly. Higher levels combine stronger defects; near 100, the scan is heavily distorted and much of its ridge structure can become difficult to distinguish. This is a severity control, not a measured percentage of damaged area, lost information or recognition error. Spatial dimensions and displacement use the clean enrollment's measured ridge width R.

A changed severity or **Randomize** produces a new time-seeded variation. The seed is retained in the test record rather than displayed in the controls. Each preview is generated from the original image, so changes never accumulate. The server applies these defects before selecting the circular capture, and feature extraction uses the exact pixels shown in the preview. Changing the preview invalidates old signing preparations and overlays while retaining the enrolled Merkle set.

**Test altered scan** prepares the currently displayed fragment, searches its candidate keys and verifies any resulting signature in one operation. The manual **Prepare features** → **Sign message** flow remains available to inspect points before searching. **Verify signature** checks an existing authorization against the message; it does not extract a new fingerprint.

Public test records include the artifact level, seed, clean ridge-width scale, generator version and grayscale-image SHA-256 for reproduction. Preview revisions bind preparations to a specific image; stale preparations are rejected.

## Parameters and outcomes

Each number field is paired with a slider, including circle coordinates and computation limits. Sliders use the displayed setting's bounds and step; fractional priority and spatial controls move in steps of 0.01 while their number fields retain exact values. Moving a slider changes only that parameter. The controls share the existing validation and become unavailable during computation; changing them does not automatically launch analysis or generate keys.

The stand exposes minimum subset size, minimum feature priority, enrollment pool multiplier, search radius in R, selected combinations per region, minimum diameter fraction, minimum anchor count, pairwise anchor separation fraction, grid uncertainty steps, enrollment memory budget, attempt limit and search-work limit. The spatial fractions use the enrolled bounds' shorter side; the summary shows their resolved R values. These controls determine geometric admission and search cost, not a calibrated cryptographic strength in bits. It uses the same library mechanisms as the command-line examples. Each point must have final `priority >= min_priority` in both enrollment and signing; equality is included. Filtering happens before regions, and low-priority points never refill a pool. Too few eligible points, an anchor count that cannot fit, or an uncovered selected pool point produces an explicit error. The weight curve remains `0.1 + 0.9 × priority²` for eligible points, subject to geometric admission.

**Enrollment memory budget** defaults to **2 GiB** and accepts **1–10 GiB** under Computation limits. Preparation uses a conservative peak-memory estimate with reserved headroom to derive consistent limits for baseline candidates, retained leaves and package size. It shows the estimated peak and supported candidate limit before key generation. If the complete requested baseline set exceeds these limits, preparation fails; it never silently truncates the set. This is an estimate, not an operating-system memory cap: actual process memory may vary.

The estimate reserves 512 MiB for working data, then adds 512 bytes per planned baseline candidate and 96 bytes per leaf in the already retained enrollment. Without a previous set, the default budget permits 3,145,728 candidates; 10 GiB permits 19,922,944. Encoded package limits grow with that allowance, and temporary tree and byte-buffer copies are released before package readback. The signing uncertainty product is generated on demand and does not count as retained enrollment keys.

The attempt limit defaults to **1,000,000,000 candidate keys** and accepts values from 1 through that maximum. Every tested positional variant consumes a key attempt. Search remains lazy: this capacity does not allocate or generate a billion candidates upfront. The separate work budget counts preparation, subset enumeration, deduplication, spatial checks, geometry and positional traversal, so it can stop a search before the key-attempt limit is reached. A rejected subset consumes work but no key attempts. Both limits retain the **incomplete** outcome; neither means that all possible candidates were tested.

Preparation reuses one compact analyzed capture when the source revision, exact circle and extraction settings are unchanged. Changing region geometry, priority cutoff or work limits rebuilds the selection/search without reprocessing the pixels. A different circle or altered-image revision requires fresh analysis. The cache holds features and their crop offset, not a full diagnostic report or additional image copies.

Changing enrollment parameters, including minimum feature priority, requires a newly prepared and registered set. The cutoff is part of the key profile, not a display setting. Changing only the enrollment memory budget invalidates an enrollment preparation, but does not change the key profile or invalidate an existing registered set. Changing the search-work limit requires preparing the fragment again; the attempt limit can change before signing. The interface hides outdated overlays and the server rejects stale preparation IDs. Operations run one at a time while the page reports their current stage.

- **Matched:** a member key was found, and both signature and Merkle proof were verified.
- **No match:** the configured regional search was exhausted without a member.
- **Incomplete:** an attempt or search-work limit stopped signing. This is not a completed no-match result. Enrollment baseline counts that exceed the memory-derived limits are rejected during preparation, before a package is created.
- **Error:** invalid input, insufficient usable data or another failed operation; the page retains the error message.

The root identifies an enrolled set. It is distinct from the Ed25519 public key of the member used for a signature. Signing searches only regional subsets, so spatially excluded combinations can be missed. The anchor policy uses profile v6 and is retained inside container-v2 `.grx` packages; signing reads the saved policy. Recreate earlier test enrollments rather than mixing the interpretations.

## Local data

Successful enrollment writes `<root>.grx` and a public preparation record under **`reports/fingerprint-lab/`**. The package retains canonical public reconstruction settings, including both enrolled spatial distances in R and the anchor count, bound to the context's profile ID. Signing writes its public result record there. These artifacts survive ordinary fingerprint-report refreshes. The active enrollment and preparations are held in server memory; after a server restart, create a new test enrollment. Private keys are transient and are not exported.

The stand binds to loopback and requires its session token for mutations. It accepts catalog image IDs and bounded numeric controls; the HTTP API does not accept filesystem paths. Each enrollment uses a new public context and salt, so its root can differ between runs. The test suite is HKDF-SHA256/Ed25519 with no independent secret. These demonstrations use a synthetic fixture and selected research scans; they do not qualify repeatability across independent scanner captures or biometric key entropy.

## Checked example

With **profile v6**, `min_priority = 0.5` and the initial anchor policy, the full fixture supplies 30 eligible features and 48 valid baseline candidates/leaves. The enrolled limits are **D = 362 px = 21.520984 R**, **k = 3** and **d = 181 px = 10.760492 R**. Circular captures centered at `(543, 724)` gave these results:

| Radius | Eligible points | Regions | Key attempts | Result |
|---|---:|---:|---:|---|
| 500 px | 11 | 4 | 8 | Matched; signature and Merkle proof verified. |
| 150 px | 0 | — | — | Preparation rejected: insufficient eligible points. |

The 500 px capture measured R = 16.366458 px and reused both saved distances: D = **352.222270 px**, d = **176.111135 px**. Matching IDs `30, 101, 110` form a triangle with diameter **526.476020 px** and minimum pair separation **249.735860 px**. An independent anchor-subset check confirmed admission. All 11 eligible points were visible. Changing the signed message failed verification; restoring it passed. The unsuccessful small capture changed neither the root nor its requirements.

Inspect the local [stand preview](../reports/fingerprint-lab/preview.png) and current run records. These are observations on the synthetic fixture, not acceptance rates across captures.
