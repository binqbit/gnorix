const $ = (id) => document.getElementById(id);
const NS = "http://www.w3.org/2000/svg";
const MIN_VISIBLE_PRIORITY = 0.25;
const configFields = {
    min_points: "min-points",
    pool_multiplier: "pool-multiplier",
    radius_units: "radius-units",
    combinations_per_region: "per-region",
    uncertainty_steps: "uncertainty",
};
const integerFields = new Set([
    "min_points",
    "pool_multiplier",
    "combinations_per_region",
    "uncertainty_steps",
]);
const state = {
    server: null,
    mode: "enrollment",
    circle: null,
    region: null,
    initialized: false,
    busy: false,
    drag: null,
    polling: false,
    artifactLevel: 0,
    artifactRandomizeQueued: false,
    artifactRequest: null,
    artifactError: null,
    imageKey: null,
    imageObjectUrl: null,
    imageLoadingKey: null,
    imageAbort: null,
    imageRequest: 0,
};
const colors = [
    "#b8ff73",
    "#43e4b0",
    "#e5cf68",
    "#7fcfe8",
    "#d7a9ff",
    "#ffbd81",
];
const format = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 });
let pollTimer;
let refreshSequence = 0;
let artifactTimer;

function countLabel(count, singular, plural = `${singular}s`) {
    return `${format.format(count)} ${count === 1 ? singular : plural}`;
}

function readConfig() {
    const config = {};
    for (const [key, id] of Object.entries(configFields))
        config[key] = Number($(id).value);
    return config;
}

function sameConfig(first, second) {
    return (
        Boolean(first && second) &&
        Object.keys(configFields).every((key) => first[key] === second[key])
    );
}

function sameCircle(first, second) {
    return (
        Boolean(first && second) &&
        ["x", "y", "radius"].every(
            (key) => Math.abs(first[key] - second[key]) < 0.01,
        )
    );
}

function validConfig() {
    for (const [key, id] of Object.entries(configFields)) {
        const input = $(id);
        const value = Number(input.value);
        if (
            !input.checkValidity() ||
            !Number.isFinite(value) ||
            (integerFields.has(key) && !Number.isInteger(value))
        )
            return false;
    }
    const config = readConfig();
    return (
        config.combinations_per_region >=
        config.min_points * (config.pool_multiplier - 1) + 1
    );
}

function validSearchControls() {
    return ["work-limit", "attempt-limit"].every((id) => {
        const input = $(id);
        return input.checkValidity() && Number.isInteger(Number(input.value));
    });
}

function payloadBytes() {
    return new TextEncoder().encode($("payload").value);
}

function validPayload() {
    return payloadBytes().length <= 16 * 1024;
}

function payloadMatches(authorization) {
    if (!authorization) return false;
    const encoded = Array.from(payloadBytes(), (byte) =>
        byte.toString(16).padStart(2, "0"),
    ).join("");
    return encoded === authorization.action.payload;
}

function artifactImageKey(artifact) {
    return `artifact:${artifact.revision}`;
}

function artifactReady() {
    const artifact = state.server?.artifact;
    return Boolean(
        artifact &&
            artifact.level === state.artifactLevel &&
            !state.artifactRandomizeQueued &&
            !state.artifactRequest &&
            !state.artifactError &&
            state.imageKey === artifactImageKey(artifact),
    );
}

function desiredImage() {
    if (!state.server) return null;
    if (state.mode === "enrollment") {
        return { key: "original", url: state.server.image.url };
    }
    const artifact = state.server.artifact;
    if (
        !artifact ||
        artifact.level !== state.artifactLevel ||
        state.artifactRandomizeQueued ||
        state.artifactRequest
    )
        return null;
    return { key: artifactImageKey(artifact), url: artifact.image_url };
}

async function syncImage() {
    const desired = desiredImage();
    if (
        !desired ||
        desired.key === state.imageKey ||
        desired.key === state.imageLoadingKey
    )
        return;
    state.imageAbort?.abort();
    const controller = new AbortController();
    state.imageAbort = controller;
    state.imageLoadingKey = desired.key;
    const request = ++state.imageRequest;
    let objectUrl;
    try {
        const response = await fetch(desired.url, {
            cache: "no-store",
            signal: controller.signal,
        });
        if (!response.ok)
            throw new Error("The scan preview could not be loaded.");
        objectUrl = URL.createObjectURL(await response.blob());
        const image = new Image();
        image.src = objectUrl;
        await image.decode();
        if (
            request !== state.imageRequest ||
            desiredImage()?.key !== desired.key
        ) {
            URL.revokeObjectURL(objectUrl);
            return;
        }
        const previous = state.imageObjectUrl;
        $("source-image").setAttribute("href", objectUrl);
        state.imageObjectUrl = objectUrl;
        state.imageKey = desired.key;
        if (previous) URL.revokeObjectURL(previous);
    } catch (error) {
        if (objectUrl) URL.revokeObjectURL(objectUrl);
        if (error.name !== "AbortError" && request === state.imageRequest)
            feedback(error.message);
    } finally {
        if (request === state.imageRequest) {
            state.imageLoadingKey = null;
            render();
        }
    }
}

function artifactJobFinished() {
    const request = state.artifactRequest;
    const job = state.server?.job;
    if (!request?.jobId || job?.id !== request.jobId || job.state === "running")
        return;
    state.artifactRequest = null;
    if (job.state === "error") {
        state.artifactError = job.error || "Artifact generation failed.";
        feedback(state.artifactError);
        return;
    }
    scheduleArtifacts(0);
}

function scheduleArtifacts(delay = 200) {
    clearTimeout(artifactTimer);
    artifactTimer = setTimeout(updateArtifacts, delay);
}

async function updateArtifacts() {
    if (
        !state.server ||
        state.mode !== "signing" ||
        !enrollmentCurrent() ||
        state.artifactRequest ||
        state.polling ||
        state.server.job?.state === "running"
    )
        return;
    if (
        state.server.artifact.level === state.artifactLevel &&
        !state.artifactRandomizeQueued
    ) {
        syncImage();
        return;
    }
    const request = {
        level: state.artifactLevel,
        randomize: state.artifactRandomizeQueued,
        jobId: null,
    };
    state.artifactRequest = request;
    state.artifactRandomizeQueued = false;
    state.artifactError = null;
    render();
    try {
        const response = await fetch("/api/artifacts", {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "X-Gnorix-Token": state.server.token,
            },
            body: JSON.stringify({
                level: request.level,
                randomize: request.randomize,
            }),
        });
        const result = await response.json();
        if (!response.ok)
            throw new Error(result.error || "Artifact generation failed.");
        request.jobId = result.job_id;
        await refresh();
    } catch (error) {
        state.artifactRequest = null;
        state.artifactError = error.message;
        feedback(error.message);
        render();
    }
}

function editArtifacts(randomize = false) {
    state.artifactLevel = Number($("artifact-level").value);
    state.artifactRandomizeQueued ||= randomize;
    state.artifactError = null;
    state.region = null;
    feedback("");
    render();
    scheduleArtifacts();
}

function currentPrepared() {
    const prepared = state.server?.prepared;
    if (
        !prepared ||
        prepared.kind !== state.mode ||
        !sameConfig(prepared.config, readConfig())
    )
        return null;
    if (prepared.work_limit !== Number($("work-limit").value)) return null;
    if (state.mode === "enrollment" && state.imageKey !== "original")
        return null;
    if (
        state.mode === "signing" &&
        (!sameCircle(prepared.selection, state.circle) ||
            !artifactReady() ||
            prepared.artifact_revision !== state.server.artifact.revision)
    )
        return null;
    return prepared;
}

function enrollmentCurrent() {
    return sameConfig(state.server?.enrollment?.config, readConfig());
}

function svgElement(tag, attributes, title) {
    const element = document.createElementNS(NS, tag);
    for (const [key, value] of Object.entries(attributes))
        element.setAttribute(key, String(value));
    if (title) {
        const child = document.createElementNS(NS, "title");
        child.textContent = title;
        element.append(child);
    }
    return element;
}

function priorityColor(priority) {
    const fraction = Math.max(0, Math.min(1, priority));
    const stops = [
        [217, 67, 85],
        [236, 132, 61],
        [220, 191, 59],
        [184, 230, 81],
        [125, 255, 105],
    ];
    const sample = Math.min(255, Math.floor(fraction * 256)) / 255;
    const position = sample * (stops.length - 1);
    const index = Math.min(stops.length - 2, Math.floor(position));
    const blend = position - index;
    const channels = stops[index].map((channel, component) =>
        Math.round(channel + (stops[index + 1][component] - channel) * blend)
            .toString(16)
            .padStart(2, "0"),
    );
    return `#${channels.join("")}`;
}

function priorityOpacity(priority) {
    return 0.4 + 0.5 * Math.sqrt(Math.max(0, Math.min(1, priority)));
}

function featureMarker(point, attributes, title) {
    const { x, y, kind } = point;
    const size = 7;
    switch (kind) {
        case "ending":
            return svgElement(
                "circle",
                { cx: x, cy: y, r: size, ...attributes },
                title,
            );
        case "bifurcation":
            return svgElement(
                "polygon",
                {
                    points: `${x},${y - size} ${x + size},${y + size} ${x - size},${y + size}`,
                    ...attributes,
                },
                title,
            );
        case "junction":
            return svgElement(
                "polygon",
                {
                    points: `${x},${y - size} ${x + size},${y} ${x},${y + size} ${x - size},${y}`,
                    ...attributes,
                },
                title,
            );
        case "isolated":
            return svgElement(
                "rect",
                {
                    x: x - size,
                    y: y - size,
                    width: size * 2,
                    height: size * 2,
                    ...attributes,
                },
                title,
            );
        case "cycle":
            return svgElement(
                "path",
                {
                    d: `M${x - 2.5},${y - size}H${x + 2.5}V${y - 2.5}H${x + size}V${y + 2.5}H${x + 2.5}V${y + size}H${x - 2.5}V${y + 2.5}H${x - size}V${y - 2.5}H${x - 2.5}Z`,
                    ...attributes,
                },
                title,
            );
        default:
            throw new Error(`Unsupported feature kind: ${kind}`);
    }
}

function drawSelection() {
    const visible = state.mode === "signing" && state.circle;
    $("circle-control").style.display = visible ? "" : "none";
    $("outside-shade").style.display = visible ? "" : "none";
    if (!visible) return;
    const { x, y, radius } = state.circle;
    for (const id of ["selection-circle", "mask-hole"]) {
        $(id).setAttribute("cx", x);
        $(id).setAttribute("cy", y);
        $(id).setAttribute("r", radius);
    }
    $("selection-center").setAttribute("cx", x);
    $("selection-center").setAttribute("cy", y);
    $("radius-handle").setAttribute("cx", x + radius);
    $("radius-handle").setAttribute("cy", y);
    $("circle-x").value = Math.round(x);
    $("circle-y").value = Math.round(y);
    $("circle-radius").value = Math.round(radius);
    $("radius-value").textContent = `${format.format(radius)} px`;
}

function drawEvidence(prepared) {
    const regionsLayer = $("regions-layer"),
        pointsLayer = $("points-layer"),
        diagnosticsLayer = $("diagnostics-layer");
    regionsLayer.replaceChildren();
    pointsLayer.replaceChildren();
    diagnosticsLayer.replaceChildren();
    const regionList = $("region-list");
    regionList.replaceChildren();
    $("feature-filter").textContent =
        `Priority ≥ ${MIN_VISIBLE_PRIORITY} · display only`;
    if (!prepared) {
        $("scale-value").textContent = "R — not measured yet";
        return;
    }
    const regions = prepared.regions ?? [];
    if (
        state.region !== null &&
        !regions.some((region) => region.id === state.region)
    )
        state.region = null;
    const selected = regions.find((region) => region.id === state.region);
    const pool = new Set(
        (selected ? [selected] : regions).flatMap(
            (region) => region.pool ?? [],
        ),
    );
    const result = state.server.result;
    const isCurrentResult =
        !result?.prepared_id || result.prepared_id === prepared.id;
    const matched = new Set(
        result?.status === "matched" &&
        state.mode === "signing" &&
        isCurrentResult
            ? (result.feature_ids ?? [])
            : [],
    );
    for (const region of regions) {
        const index = regions.indexOf(region),
            active = state.region === null || state.region === region.id;
        const color = colors[index % colors.length];
        regionsLayer.append(
            svgElement("circle", {
                cx: region.x,
                cy: region.y,
                r: region.radius,
                fill: color,
                "fill-opacity": active ? 0.035 : 0.005,
                stroke: color,
                "stroke-opacity": active ? 0.78 : 0.2,
                "stroke-width": active ? 1.5 : 1,
                "vector-effect": "non-scaling-stroke",
                "pointer-events": "none",
            }),
        );
        if (region.radius > region.initial_radius + 0.01)
            regionsLayer.append(
                svgElement("circle", {
                    cx: region.x,
                    cy: region.y,
                    r: region.initial_radius,
                    fill: "none",
                    stroke: color,
                    "stroke-opacity": 0.35,
                    "stroke-dasharray": "5 5",
                    "stroke-width": 1,
                    "vector-effect": "non-scaling-stroke",
                    "pointer-events": "none",
                }),
            );
    }
    const points = prepared.points ?? [];
    const visiblePoints = points.filter(
        (point) => point.priority >= MIN_VISIBLE_PRIORITY,
    );
    for (const point of visiblePoints) {
        const included = pool.has(point.id);
        const isMatched = matched.has(point.id);
        const color = priorityColor(point.priority);
        pointsLayer.append(
            featureMarker(
                point,
                {
                    fill: color,
                    stroke: color,
                    opacity: priorityOpacity(point.priority),
                    "stroke-width": 0.3,
                    "vector-effect": "non-scaling-stroke",
                    "data-feature-id": point.id,
                    "data-x": point.x,
                    "data-y": point.y,
                    "data-kind": point.kind,
                    "data-priority": point.priority,
                    "data-matched": isMatched,
                    "data-selected-pool": included,
                },
                `#${point.id} · ${point.kind} · priority ${format.format(point.priority * 100)}%${isMatched ? " · matched" : included ? " · selected pool" : ""}`,
            ),
        );
        if (included) {
            diagnosticsLayer.append(
                svgElement("path", {
                    d: `M${point.x - 4},${point.y + 10}H${point.x + 4}`,
                    fill: "none",
                    stroke: "#a8b9b0",
                    "stroke-width": 0.8,
                    "vector-effect": "non-scaling-stroke",
                    "pointer-events": "none",
                    "data-indicator": "selected-pool",
                    "data-for-feature-id": point.id,
                }),
            );
        }
        if (isMatched) {
            diagnosticsLayer.append(
                svgElement("circle", {
                    cx: point.x,
                    cy: point.y,
                    r: 14,
                    fill: "none",
                    stroke: "#ffffff",
                    "stroke-width": 1.5,
                    "vector-effect": "non-scaling-stroke",
                    "pointer-events": "none",
                    "data-indicator": "matched",
                    "data-for-feature-id": point.id,
                }),
            );
        }
    }
    const options = [
        { id: null, label: `All regions · ${regions.length}` },
        ...regions.map((region) => ({
            id: region.id,
            label: `#${region.id + 1} · ${region.pool?.length ?? 0}/${region.available_points} points`,
        })),
    ];
    for (const option of options) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = `region-chip${option.id === state.region ? " active" : ""}`;
        button.textContent = option.label;
        button.setAttribute("aria-pressed", String(option.id === state.region));
        button.addEventListener("click", () => {
            state.region = option.id;
            drawEvidence(currentPrepared());
        });
        regionList.append(button);
    }
    $("scale-value").textContent =
        `R = ${format.format(prepared.unit_px)} px · ${visiblePoints.length}/${countLabel(points.length, "feature")} shown`;
}

function renderResult(prepared) {
    const result = state.server?.result;
    const visible =
        result &&
        prepared &&
        state.mode === "signing" &&
        (!result.prepared_id || result.prepared_id === prepared.id);
    $("result-panel").hidden = !visible;
    $("verify").disabled =
        state.busy || !result?.authorization || !validPayload();
    if (!visible) return;
    const matched = result.status === "matched";
    $("result-panel").classList.toggle(
        "warning",
        !matched || result.verified === false,
    );
    const statuses = {
        matched: ["Key found", "MATCHED"],
        no_match: ["No match found", "NO MATCH"],
        incomplete: ["Search incomplete", "INCOMPLETE"],
        error: ["Experiment failed", "ERROR"],
    };
    const [title, badge] = statuses[result.status] ?? statuses.error;
    $("result-title").textContent = title;
    $("result-badge").textContent = badge;
    $("result-message").textContent = result.message ?? "";
    $("result-attempts").textContent = format.format(result.attempts ?? 0);
    $("result-features").textContent = String(result.feature_ids?.length ?? 0);
    $("verified-label").textContent =
        result.authorization && !payloadMatches(result.authorization)
            ? "The message differs from the signed message"
            : result.verified === true
              ? "✓ Signature and Merkle proof verified"
              : result.verified === false && result.authorization
                ? "✕ Signature verification failed"
                : "No signature";
    document.querySelector(".result-data").hidden = !result.authorization;
    document.querySelector(".authorization-details").hidden =
        !result.authorization;
    if (result.authorization) {
        const authorization = result.authorization;
        $("public-key").textContent = authorization.public_key;
        $("signature").textContent = authorization.signature;
        $("proof-json").textContent = JSON.stringify(
            authorization.proof,
            null,
            2,
        );
        $("proof-meta").textContent =
            `Leaf ${authorization.proof.leaf_index} · ${countLabel(authorization.proof.leaf_count, "leaf", "leaves")} · ${countLabel(authorization.proof.siblings.length, "proof hash", "proof hashes")}`;
        $("authorization-json").textContent = JSON.stringify(
            authorization,
            null,
            2,
        );
    }
}

function render() {
    if (!state.server) return;
    const { enrollment, job } = state.server;
    const prepared = currentPrepared(),
        registered = enrollmentCurrent(),
        valid = validConfig();
    state.busy =
        state.polling ||
        Boolean(state.artifactRequest) ||
        job?.state === "running";
    $("mode-enrollment").classList.toggle(
        "active",
        state.mode === "enrollment",
    );
    $("mode-signing").classList.toggle("active", state.mode === "signing");
    $("circle-settings").hidden = state.mode !== "signing";
    $("reset-circle").hidden = state.mode !== "signing";
    $("artifact-settings").hidden = state.mode !== "signing";
    $("artifact-test-actions").hidden = state.mode !== "signing";
    $("artifact-value").textContent =
        state.artifactLevel === 0
            ? "Original · 0"
            : `${state.artifactLevel} / 100`;
    const artifactControlsLocked =
        !registered || (state.busy && !state.artifactRequest);
    $("artifact-level").disabled = artifactControlsLocked;
    $("artifact-randomize").disabled = artifactControlsLocked;
    $("artifact-status").textContent = !registered
        ? "Create a clean Merkle set before altering a scan."
        : state.artifactError
          ? state.artifactError
          : !artifactReady()
            ? "Updating the exact scan preview. Previous feature markers are hidden."
            : "This exact preview is used for extraction. Change severity or select Randomize for a new variation.";
    const displayReady =
        state.mode === "enrollment"
            ? state.imageKey === "original"
            : artifactReady();
    $("image-stage").classList.toggle("preview-pending", !displayReady);
    $("action-title").textContent =
        state.mode === "enrollment"
            ? "Prepare a Merkle set"
            : "Sign with a fragment";
    $("mode-help").textContent =
        state.mode === "enrollment"
            ? "Inspect the regions and points first. Create the set when you are ready."
            : "Choose a circle, prepare its features, then search for a key in the enrolled set.";
    $("execute").textContent =
        state.mode === "enrollment" ? "Create Merkle set" : "Sign message";
    const signingReady =
        registered &&
        validSearchControls() &&
        validPayload() &&
        artifactReady();
    const consumed =
        prepared && state.mode === "signing" && state.server.result;
    if (consumed)
        $("execute").textContent = "Prepare again to sign another message";
    $("execute").disabled =
        state.busy ||
        !prepared ||
        consumed ||
        (state.mode === "signing" && !signingReady);
    $("prepare").disabled =
        state.busy ||
        !displayReady ||
        !valid ||
        (state.mode === "signing" && !signingReady);
    $("test-altered-scan").disabled = state.busy || !valid || !signingReady;
    for (const id of [
        "mode-enrollment",
        "mode-signing",
        "reset-circle",
        "circle-x",
        "circle-y",
        "circle-radius",
        ...Object.values(configFields),
        "work-limit",
        "attempt-limit",
    ])
        $(id).disabled = state.busy;
    $("payload").disabled = state.busy;
    $("config-status").textContent = enrollment
        ? registered
            ? "Matches enrolled set"
            : "Settings changed"
        : "Before enrollment";
    $("config-status").classList.toggle(
        "dirty",
        Boolean(enrollment && !registered),
    );
    $("config-help").textContent = !valid
        ? "Cover the pool: combinations / region ≥ n × (multiplier − 1) + 1. Check the value limits too."
        : enrollment && !registered
          ? "These settings differ from the enrolled set. Prepare and create a new set, or restore the previous settings."
          : "R is the measured mean dark-ridge width. Changing these settings requires a new Merkle set.";
    $("merkle-root").textContent = enrollment?.root ?? "No set created yet";
    $("leaf-count").textContent = enrollment
        ? countLabel(enrollment.leaf_count, "key")
        : "— keys";
    $("root-state").textContent = enrollment
        ? registered
            ? "Active set"
            : "Different settings"
        : "Awaiting enrollment";
    $("copy-root").disabled = !enrollment;
    $("step-prepare").classList.toggle("done", Boolean(prepared));
    $("step-enroll").classList.toggle("done", Boolean(enrollment));
    $("step-sign").classList.toggle(
        "done",
        state.server.result?.status === "matched",
    );
    let notice =
        state.mode === "enrollment"
            ? "Prepare the image to see its detected features."
            : registered
              ? "Drag the circle, then prepare the selected fragment."
              : "First create a Merkle set with the current settings.";
    if (prepared)
        notice =
            state.mode === "enrollment"
                ? `${countLabel(prepared.regions.length, "region")} · ${countLabel(prepared.combination_count, "combination")} · ${countLabel(prepared.variant_count ?? 0, "positional variant")}`
                : state.server.result?.status === "matched"
                  ? `Key reconstructed with ${countLabel(state.server.result.feature_ids.length, "feature")} · white rings mark visible matches`
                  : `${countLabel(prepared.points.length, "feature")} in the selected fragment · ready to search`;
    if (state.mode === "signing" && registered && !artifactReady())
        notice = "Updating scan preview · prepare or test when ready";
    if (state.mode === "enrollment" && !displayReady)
        notice = "Loading the original scan…";
    $("viewer-notice").textContent = notice;
    const status = $("job-status");
    status.classList.toggle("busy", state.busy);
    status.classList.toggle("error", job?.state === "error");
    status.lastElementChild.textContent = state.busy
        ? state.artifactRequest
            ? "Updating altered scan…"
            : job?.phase || "Starting computation…"
        : job?.state === "error"
          ? job.error || "Computation failed"
          : prepared
            ? "Preparation complete"
            : "Ready to prepare";
    drawSelection();
    drawEvidence(prepared);
    renderResult(prepared);
}

function feedback(message) {
    $("feedback").hidden = !message;
    $("feedback").textContent = message || "";
}

async function refresh() {
    const sequence = ++refreshSequence;
    try {
        const response = await fetch("/api/state", { cache: "no-store" });
        if (!response.ok) throw new Error(`Server returned ${response.status}`);
        const next = await response.json();
        if (sequence !== refreshSequence) return;
        state.server = next;
        if (!state.initialized) {
            const { image } = state.server;
            const defaults =
                state.server.prepared?.config ??
                state.server.enrollment?.config ??
                state.server.defaults;
            for (const [key, id] of Object.entries(configFields))
                if (defaults[key] !== undefined) $(id).value = defaults[key];
            const { width, height } = image;
            $("fingerprint").setAttribute("viewBox", `0 0 ${width} ${height}`);
            for (const id of [
                "source-image",
                "outside-shade",
                "mask-background",
            ]) {
                $(id).setAttribute("width", width);
                $(id).setAttribute("height", height);
            }
            $("source-image").setAttribute("href", image.url);
            $("image-size").textContent = `${width} × ${height} px`;
            $("circle-x").max = width - 1;
            $("circle-y").max = height - 1;
            $("circle-radius").max = Math.ceil(Math.hypot(width, height));
            state.circle = {
                x: Math.round(width / 2),
                y: Math.round(height / 2),
                radius: Math.round(Math.min(width, height) * 0.34),
            };
            state.artifactLevel = state.server.artifact?.level ?? 0;
            $("artifact-level").value = state.artifactLevel;
            if (state.server.prepared) {
                state.mode = state.server.prepared.kind;
                $("work-limit").value = state.server.prepared.work_limit;
                if (state.server.prepared.selection)
                    state.circle = { ...state.server.prepared.selection };
            }
            const payload = state.server.result?.authorization?.action?.payload;
            if (
                typeof payload === "string" &&
                /^(?:[0-9a-f]{2})*$/.test(payload)
            ) {
                const bytes = Uint8Array.from(
                    payload.match(/.{2}/g) ?? [],
                    (byte) => parseInt(byte, 16),
                );
                $("payload").value = new TextDecoder().decode(bytes);
            }
            state.initialized = true;
        }
        $("connection").textContent = "● Local server";
        $("connection").classList.remove("offline");
        artifactJobFinished();
        render();
        syncImage();
    } catch (error) {
        if (sequence !== refreshSequence) return;
        $("connection").textContent = "Server unavailable";
        $("connection").classList.add("offline");
        feedback(error.message);
        $("prepare").disabled = true;
        $("execute").disabled = true;
        $("verify").disabled = true;
        $("test-altered-scan").disabled = true;
    }
    clearTimeout(pollTimer);
    pollTimer = setTimeout(
        refresh,
        state.server?.job?.state === "running" ? 500 : 2500,
    );
}

async function submit(path, body) {
    if (state.busy) return;
    if (
        (path === "/api/sign" ||
            path === "/api/verify" ||
            path === "/api/test") &&
        !validPayload()
    ) {
        feedback("The UTF-8 message exceeds 16 KiB. Shorten the text.");
        return;
    }
    feedback("");
    state.polling = true;
    render();
    try {
        const response = await fetch(path, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "X-Gnorix-Token": state.server.token,
            },
            body: JSON.stringify(body),
        });
        const result = await response.json();
        if (!response.ok)
            throw new Error(
                result.error || `Server returned ${response.status}`,
            );
        state.region = null;
    } catch (error) {
        feedback(error.message);
    } finally {
        state.polling = false;
        await refresh();
    }
}

function setMode(mode) {
    if (state.busy) return;
    state.mode = mode;
    state.region = null;
    feedback("");
    render();
    syncImage();
    if (mode === "signing") scheduleArtifacts(0);
}
$("mode-enrollment").addEventListener("click", () => setMode("enrollment"));
$("mode-signing").addEventListener("click", () => setMode("signing"));
$("prepare").addEventListener("click", () =>
    submit("/api/prepare", {
        kind: state.mode,
        config: readConfig(),
        selection: state.mode === "signing" ? state.circle : null,
        artifact_revision:
            state.mode === "signing" ? state.server.artifact.revision : null,
        work_limit: Number($("work-limit").value),
    }),
);
$("execute").addEventListener("click", () => {
    const prepared = currentPrepared();
    if (!prepared) return;
    if (state.mode === "enrollment")
        submit("/api/enroll", { prepared_id: prepared.id });
    else
        submit("/api/sign", {
            prepared_id: prepared.id,
            payload: $("payload").value,
            attempt_limit: Number($("attempt-limit").value),
            work_limit: Number($("work-limit").value),
        });
});
$("verify").addEventListener("click", () =>
    submit("/api/verify", { payload: $("payload").value }),
);
$("artifact-level").addEventListener("input", () => editArtifacts());
$("artifact-randomize").addEventListener("click", () => editArtifacts(true));
$("test-altered-scan").addEventListener("click", () => {
    if (!artifactReady()) return;
    submit("/api/test", {
        config: readConfig(),
        selection: state.circle,
        artifact_revision: state.server.artifact.revision,
        payload: $("payload").value,
        attempt_limit: Number($("attempt-limit").value),
        work_limit: Number($("work-limit").value),
    });
});
for (const id of [
    ...Object.values(configFields),
    "work-limit",
    "attempt-limit",
])
    $(id).addEventListener("input", () => {
        state.region = null;
        feedback("");
        render();
    });
$("payload").addEventListener("input", () => {
    $("payload").setCustomValidity(
        validPayload() ? "" : "The UTF-8 message exceeds 16 KiB",
    );
    feedback(
        validPayload()
            ? ""
            : "The UTF-8 message exceeds 16 KiB. Shorten the text.",
    );
    render();
});
for (const [id, key] of [
    ["circle-x", "x"],
    ["circle-y", "y"],
    ["circle-radius", "radius"],
])
    $(id).addEventListener("input", () => {
        const value = Number($(id).value);
        if (!Number.isFinite(value)) return;
        state.circle[key] = value;
        constrainCircle();
        state.region = null;
        render();
    });
$("reset-circle").addEventListener("click", () => {
    const image = state.server.image;
    state.circle = {
        x: Math.round(image.width / 2),
        y: Math.round(image.height / 2),
        radius: Math.round(Math.min(image.width, image.height) * 0.34),
    };
    state.region = null;
    render();
});

function constrainCircle() {
    if (!state.server || !state.circle) return;
    const { width, height } = state.server.image;
    state.circle.x = Math.round(
        Math.max(0, Math.min(width - 1, state.circle.x)),
    );
    state.circle.y = Math.round(
        Math.max(0, Math.min(height - 1, state.circle.y)),
    );
    state.circle.radius = Math.round(
        Math.max(16, Math.min(Math.hypot(width, height), state.circle.radius)),
    );
}

function eventPoint(event) {
    const point = $("fingerprint").createSVGPoint();
    point.x = event.clientX;
    point.y = event.clientY;
    return point.matrixTransform($("fingerprint").getScreenCTM().inverse());
}
$("fingerprint").addEventListener("pointerdown", (event) => {
    if (
        state.mode !== "signing" ||
        state.busy ||
        !state.circle ||
        event.button !== 0
    )
        return;
    const point = eventPoint(event);
    const distance = Math.hypot(
        point.x - state.circle.x,
        point.y - state.circle.y,
    );
    const scale = $("fingerprint").getScreenCTM().a;
    const resize =
        event.target.id === "radius-handle" ||
        Math.abs(distance - state.circle.radius) * scale < 12;
    state.drag = {
        pointer: event.pointerId,
        resize,
        offsetX: point.x - state.circle.x,
        offsetY: point.y - state.circle.y,
    };
    if (!resize && distance > state.circle.radius) {
        state.circle.x = point.x;
        state.circle.y = point.y;
        state.drag.offsetX = 0;
        state.drag.offsetY = 0;
    }
    $("fingerprint").setPointerCapture(event.pointerId);
    $("selection-circle").classList.add("dragging");
    constrainCircle();
    state.region = null;
    render();
    event.preventDefault();
});
$("fingerprint").addEventListener("pointermove", (event) => {
    if (!state.drag || state.drag.pointer !== event.pointerId) return;
    const point = eventPoint(event);
    if (state.drag.resize)
        state.circle.radius = Math.hypot(
            point.x - state.circle.x,
            point.y - state.circle.y,
        );
    else {
        state.circle.x = point.x - state.drag.offsetX;
        state.circle.y = point.y - state.drag.offsetY;
    }
    constrainCircle();
    render();
});
function stopDrag() {
    state.drag = null;
    $("selection-circle").classList.remove("dragging");
}
$("fingerprint").addEventListener("pointerup", stopDrag);
$("fingerprint").addEventListener("pointercancel", stopDrag);
$("fingerprint").addEventListener("lostpointercapture", stopDrag);
$("fingerprint").addEventListener("keydown", (event) => {
    if (state.mode !== "signing" || state.busy || !state.circle) return;
    const step = event.shiftKey ? 20 : 2;
    const action = {
        ArrowLeft: () => (state.circle.x -= step),
        ArrowRight: () => (state.circle.x += step),
        ArrowUp: () => (state.circle.y -= step),
        ArrowDown: () => (state.circle.y += step),
        "+": () => (state.circle.radius += step),
        "=": () => (state.circle.radius += step),
        "-": () => (state.circle.radius -= step),
    }[event.key];
    if (action) {
        action();
        constrainCircle();
        render();
        event.preventDefault();
    }
});

async function copyText(button, text) {
    try {
        await navigator.clipboard.writeText(text);
        const old = button.textContent;
        button.textContent = "Copied";
        setTimeout(() => {
            button.textContent = old;
        }, 1400);
    } catch {
        feedback(
            "The browser blocked clipboard access. Select and copy the text manually.",
        );
    }
}
$("copy-root").addEventListener("click", (event) =>
    copyText(event.currentTarget, state.server.enrollment.root),
);
document
    .querySelectorAll("[data-copy]")
    .forEach((button) =>
        button.addEventListener("click", () =>
            copyText(button, $(button.dataset.copy).textContent),
        ),
    );
refresh();
