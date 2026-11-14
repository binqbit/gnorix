"use strict";

const $ = (id) => document.getElementById(id);
const SVG_NS = "http://www.w3.org/2000/svg";
const MAX_AMOUNT = (1n << 128n) - 1n;
const state = {
    server: null,
    wallet: null,
    connected: false,
    requestInFlight: false,
    readPromise: null,
    dismissedProofs: new Set(),
    imageKey: null,
    imageRequest: 0,
    pointsKey: null,
    captureKey: null,
};

function activeWallet() {
    return state.server?.wallets.find((wallet) => wallet.id === state.wallet) ?? null;
}

function recipientWallet() {
    return state.server?.wallets.find((wallet) => wallet.id !== state.wallet) ?? null;
}

function activeCapture() {
    const capture = state.server?.capture;
    return capture?.wallet === state.wallet ? capture : null;
}

function busy() {
    return state.requestInFlight || Boolean(state.server?.busy);
}

function unresolvedSubmission() {
    return state.server?.phase === "unknown" || ["pending", "unknown"].includes(state.server?.transaction?.status);
}

function transferFinished() {
    return !unresolvedSubmission() && ["complete", "failed"].includes(state.server?.phase);
}

function formatUnits(value, decimals = 24) {
    const digits = BigInt(value).toString().padStart(decimals + 1, "0");
    const integer = decimals ? digits.slice(0, -decimals) : digits;
    const fraction = decimals ? digits.slice(-decimals).replace(/0+$/, "") : "";
    return fraction ? `${integer}.${fraction}` : integer;
}

function compactBalance(value) {
    const full = formatUnits(value, state.server.decimals);
    const [integer, fraction] = full.split(".");
    const grouped = integer.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    return fraction ? `${grouped}.${fraction.slice(0, 6)}${fraction.length > 6 ? "…" : ""}` : grouped;
}

function parseAmount(value) {
    const decimals = state.server?.decimals ?? 24;
    const text = value.trim();
    if (text.length > 64) throw new Error("The amount is too long.");
    if (!/^\d+(?:\.\d*)?$/.test(text)) {
        throw new Error("Enter a positive decimal amount, without commas or an exponent.");
    }
    const [integer, fraction = ""] = text.split(".");
    if (fraction.length > decimals) throw new Error(`Use at most ${decimals} decimal places.`);
    const units = BigInt(integer) * 10n ** BigInt(decimals) + BigInt(fraction.padEnd(decimals, "0"));
    if (units <= 0n) throw new Error("The amount must be greater than zero.");
    if (units > MAX_AMOUNT) throw new Error("This amount is too large for the token.");
    return { units, decimal: formatUnits(units, decimals) };
}

function transferInput() {
    const wallet = activeWallet();
    const recipient = recipientWallet();
    if (!wallet || !recipient) throw new Error("Wait for both wallets to finish loading.");
    const amount = parseAmount($("amount").value);
    if (amount.units > BigInt(wallet.balance_base_units)) {
        throw new Error("The amount exceeds this wallet’s current GXT balance.");
    }
    return { wallet: wallet.id, recipient: recipient.id, amount: amount.decimal, memo: null };
}

function displayedProof() {
    const prepared = state.server?.prepared;
    if (!prepared || prepared.wallet !== state.wallet || state.dismissedProofs.has(prepared.id)) return null;
    if (prepared.capture_id !== activeCapture()?.id) return null;
    // Only an unsubmitted authorization must match the editable draft. A
    // completed attempt retains its own signed payload and receipt as evidence.
    if (state.server.phase !== "signed") return prepared;
    const recipient = recipientWallet();
    try {
        const amount = parseAmount($("amount").value);
        if (prepared.recipient !== recipient?.id || amount.units.toString() !== prepared.amount_base_units) return null;
    } catch {
        return null;
    }
    return prepared;
}

function setText(id, text) {
    const node = $(id);
    if (node.textContent !== String(text)) node.textContent = text;
}

function showFormError(message) {
    if (message && $("form-error").textContent !== message) $("status-content").scrollTop = 0;
    setText("form-error", message ?? "");
    $("form-error").hidden = !message;
}

function dismissProof() {
    if (state.server?.phase === "signed" && state.server.prepared) {
        state.dismissedProofs.add(state.server.prepared.id);
    }
    showFormError(null);
    render();
}

function setWallet(id) {
    if (busy() || unresolvedSubmission() || id === state.wallet) return;
    state.wallet = id;
    $("amount").value = "";
    showFormError(null);
    render();
}

function walletButton(wallet) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "wallet-card";
    button.dataset.walletId = wallet.id;
    const icon = document.createElement("span");
    icon.className = "wallet-icon";
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = String(wallet.id + 1).padStart(2, "0");
    const identity = document.createElement("span");
    const name = document.createElement("span");
    name.className = "wallet-name";
    name.append(document.createTextNode(wallet.name));
    const selected = document.createElement("span");
    selected.className = "active-wallet-label";
    selected.textContent = "Selected";
    name.append(selected);
    const account = document.createElement("span");
    account.className = "wallet-account";
    account.textContent = wallet.account_id;
    account.title = wallet.account_id;
    identity.append(name, account);
    const balance = document.createElement("span");
    balance.className = "wallet-balance";
    const amount = document.createElement("span");
    amount.className = "balance-value";
    const symbol = document.createElement("small");
    symbol.textContent = state.server.symbol;
    balance.append(amount, symbol);
    button.append(icon, identity, balance);
    button.addEventListener("click", () => setWallet(wallet.id));
    return button;
}

function renderWallets() {
    const wallets = state.server?.wallets ?? [];
    const container = $("wallets");
    if (!wallets.length) {
        if (!container.querySelector(".wallet-placeholder")) {
            const placeholder = document.createElement("div");
            placeholder.className = "wallet-placeholder";
            placeholder.textContent = "Loading registered wallets…";
            container.replaceChildren(placeholder);
        }
        return;
    }
    const existing = [...container.querySelectorAll("[data-wallet-id]")];
    if (existing.length !== wallets.length || existing.some((button, index) => Number(button.dataset.walletId) !== wallets[index].id)) {
        container.replaceChildren(...wallets.map(walletButton));
    }
    for (const wallet of wallets) {
        const button = container.querySelector(`[data-wallet-id="${wallet.id}"]`);
        const selected = wallet.id === state.wallet;
        button.setAttribute("aria-pressed", String(selected));
        button.disabled = busy() || unresolvedSubmission();
        button.querySelector(".active-wallet-label").hidden = !selected;
        button.querySelector(".balance-value").textContent = compactBalance(wallet.balance_base_units);
        button.querySelector(".wallet-balance").title = `${formatUnits(wallet.balance_base_units, state.server.decimals)} ${state.server.symbol}`;
        button.setAttribute("aria-label", `${wallet.name}, ${formatUnits(wallet.balance_base_units, state.server.decimals)} ${state.server.symbol}${selected ? ", selected" : ""}`);
    }
}

function renderConnection() {
    const server = state.server;
    const setupFailed = server?.phase === "failed" && !server.chain_id;
    const status = !state.connected || setupFailed ? "error" : server?.phase === "starting" ? "connecting" : "ready";
    $("connection").dataset.state = status;
    setText("connection-label", !server ? "Local server unavailable" : !state.connected ? "Connection interrupted" : setupFailed ? "Sandbox setup failed" : server.phase === "starting" ? "Preparing local sandbox" : "Sandbox connected");
    $("connection").title = server?.chain_id ? `Chain: ${server.chain_id}` : "Local NEAR sandbox";
}

async function loadFingerprint(wallet, capture) {
    const source = wallet.fingerprint;
    const key = `${wallet.id}:${source.image_url}:${capture?.id ?? ""}`;
    if (key === state.imageKey) return;
    state.imageKey = key;
    const request = ++state.imageRequest;
    $("fingerprint").toggleAttribute("hidden", true);
    $("image-placeholder").hidden = false;
    setText("image-placeholder", capture ? "Loading the actual captured image…" : "Loading this wallet’s fingerprint…");
    try {
        const image = new Image();
        image.src = capture?.image_url ?? source.image_url;
        await image.decode();
        if (request !== state.imageRequest) return;
        $("fingerprint").setAttribute("viewBox", `0 0 ${source.width} ${source.height}`);
        $("fingerprint-image").setAttribute("width", source.width);
        $("fingerprint-image").setAttribute("height", source.height);
        $("fingerprint-image").setAttribute("href", image.src);
        if (capture) {
            $("capture-image").setAttribute("width", capture.width);
            $("capture-image").setAttribute("height", capture.height);
            $("capture-image").setAttribute("href", capture.image_url);
        } else {
            $("capture-image").removeAttribute("href");
        }
        $("fingerprint").toggleAttribute("hidden", false);
        $("image-placeholder").hidden = true;
    } catch {
        if (request !== state.imageRequest) return;
        setText("image-placeholder", capture ? "Could not load this capture. The analysis result remains available below." : "Could not load this fingerprint. Refresh the page to retry.");
    }
}

function featureColor(priority) {
    if (typeof priority !== "number" || !Number.isFinite(priority)) return { color: "#b5c0b5", opacity: 0.4 };
    const value = Math.max(0, Math.min(1, priority));
    return { color: `hsl(${value * 115} 90% 65%)`, opacity: 0.4 + 0.5 * Math.sqrt(value) };
}

function svgElement(tag, attributes) {
    const element = document.createElementNS(SVG_NS, tag);
    for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
    return element;
}

function selectionArcs(x, y, radius) {
    return [0, 1, 2, 3].map((quadrant) => {
        const start = (quadrant + 0.25) * Math.PI / 2;
        const end = (quadrant + 0.75) * Math.PI / 2;
        return `M ${x + radius * Math.cos(start)} ${y + radius * Math.sin(start)} A ${radius} ${radius} 0 0 1 ${x + radius * Math.cos(end)} ${y + radius * Math.sin(end)}`;
    }).join(" ");
}

function featureMarkerSize(source) {
    const bounds = $("fingerprint").getBoundingClientRect();
    const scale = Math.min(bounds.width / source.width, bounds.height / source.height) || 1;
    const dot = Math.max(Math.max(source.width, source.height) * 0.009, 2.25 / scale);
    return { dot, selection: Math.max(dot * 2.8, 10 / scale) };
}

function insideCapture(point, capture) {
    if (!capture) return true;
    return Math.hypot(point.x - capture.region.x, point.y - capture.region.y) <= capture.region.radius;
}

function renderCaptureRegion(capture) {
    $("fingerprint").classList.toggle("has-capture", Boolean(capture));
    $("capture-image").toggleAttribute("hidden", !capture);
    if (!capture) {
        $("matched-points").removeAttribute("clip-path");
        state.captureKey = null;
        return;
    }
    $("matched-points").setAttribute("clip-path", "url(#capture-clip)");
    if (state.captureKey === capture.id) return;
    const { x, y, radius } = capture.region;
    for (const id of ["capture-clip-circle", "capture-mask-circle"]) {
        for (const [name, value] of Object.entries({ cx: x, cy: y, r: radius })) $(id).setAttribute(name, value);
    }
    $("capture-mask").setAttribute("width", capture.width);
    $("capture-mask").setAttribute("height", capture.height);
    $("outside-blur-radius").setAttribute("stdDeviation", Math.max(capture.width, capture.height) * 0.01);
    state.captureKey = capture.id;
}

function renderFeaturePoints(wallet, capture, prepared) {
    const source = capture ?? wallet.fingerprint;
    const matches = new Set(prepared ? capture.matched_feature_ids : []);
    const points = (source.points ?? []).filter((point) => insideCapture(point, capture));
    const size = featureMarkerSize(source);
    const pointsKey = JSON.stringify([wallet.id, capture?.id, prepared?.id, points, [...matches], size]);
    if (state.pointsKey !== pointsKey) {
        const markers = [];
        const dots = [];
        for (const point of points) {
            const appearance = featureColor(point.priority);
            dots.push(svgElement("circle", {
                cx: point.x, cy: point.y, r: size.dot,
                fill: appearance.color, "fill-opacity": appearance.opacity,
                class: "feature-point",
                "data-feature-id": point.id,
            }));
            if (matches.has(point.id)) {
                const arcs = selectionArcs(point.x, point.y, size.selection);
                for (const layer of ["contour", "marker"]) {
                    markers.push(svgElement("path", {
                        d: arcs,
                        class: `matched-feature-${layer}`,
                        "data-feature-id": point.id,
                    }));
                }
            }
        }
        $("matched-points").replaceChildren(...markers, ...dots);
        state.pointsKey = pointsKey;
    }
    return points.filter((point) => matches.has(point.id)).length;
}

function renderFingerprint() {
    const wallet = activeWallet();
    if (!wallet) {
        $("fingerprint").toggleAttribute("hidden", true);
        $("image-placeholder").hidden = false;
        setText("image-placeholder", "The wallet’s test fingerprint will appear here.");
        setText("fingerprint-title", "Your fingerprint");
        setText("fingerprint-tag", "Awaiting wallet");
        setText("enrollment-root", "—");
        setText("source-size", "");
        return;
    }
    const capture = activeCapture();
    const analyzing = busy() && state.server?.phase === "signing";
    const source = capture ?? wallet.fingerprint;
    void loadFingerprint(wallet, capture);
    renderCaptureRegion(capture);
    const matched = renderFeaturePoints(wallet, capture, displayedProof());
    $("fingerprint-stage").classList.toggle("is-signing", analyzing && Boolean(capture));
    $("fingerprint-stage").dataset.captureId = capture?.id ?? "";
    $("fingerprint").setAttribute("aria-label", capture ? `${wallet.name} captured region; blurred exterior is excluded from analysis` : `${wallet.name} enrollment fingerprint`);
    setText("fingerprint-title", `${wallet.name} ${capture ? "capture" : "fingerprint"}`);
    setText("source-size", `${source.width} × ${source.height}`);

    let featureLabel = `${source.feature_count} ${capture ? "capture " : ""}features`;
    let pointStatus = "Enrollment features · a new capture is used for signing";
    if (capture && source.feature_count === null) {
        featureLabel = analyzing ? "Analyzing capture" : "Capture retained";
        pointStatus = analyzing ? "Extracting features from the captured region…" : "Feature analysis was not completed";
    } else if (capture) {
        pointStatus = matched ? `${matched} matched signing points · green arcs` : `${source.feature_count} fresh features · no signing match highlighted`;
    } else if (analyzing) {
        pointStatus = "Preparing a fresh random capture…";
    }
    setText("fingerprint-tag", featureLabel);
    setText("point-status", pointStatus);
    setText("capture-description", capture ? `${(capture.pattern_fraction * 100).toFixed(1)}% of the fingerprint pattern · Minimum priority 0.1. Blur is visual; analysis uses the unchanged circular sample only.` : "Each approval analyzes a new random circular sample. The surroundings fade into blur. Minimum feature priority: 0.1.");
    setText("enrollment-root", wallet.root);
}

function renderForm() {
    const wallet = activeWallet();
    const recipient = recipientWallet();
    const prepared = transferFinished() ? null : displayedProof();
    const mutable = state.connected && wallet && recipient && !busy() && !prepared && !unresolvedSubmission();
    $("transfer-fields").disabled = !mutable;
    $("refresh-state").disabled = !state.connected || !state.server?.token || busy();
    setText("refresh-label", unresolvedSubmission() ? "Check transaction status" : "Refresh balances");
    setText("recipient-name", recipient?.name ?? "Other demo wallet");
    $("recipient-address").value = recipient?.account_id ?? "";
    setText("available-balance", wallet ? `Available ${compactBalance(wallet.balance_base_units)} GXT` : "Available —");
    setText("review-recipient", recipient ? `to ${recipient.name} · ${recipient.account_id}` : "Choose the recipient before signing.");
    let valid = false;
    try {
        const input = transferInput();
        valid = true;
        setText("review-amount", `${input.amount} GXT`);
    } catch {
        setText("review-amount", $("amount").value.trim() ? "Check the transfer amount" : "Enter an amount");
    }
    if (prepared) setText("review-amount", `${prepared.amount} GXT`);
    $("sign-button").hidden = Boolean(prepared);
    $("sign-button").disabled = !mutable || !valid;
    setText("sign-label", state.server?.phase === "signing" && busy() ? "Reconstructing…" : "Approve & sign");
    const canSubmit = prepared?.verified && state.server?.phase === "signed" && !busy() && state.connected;
    $("submit-button").hidden = !prepared;
    setText("submit-label", unresolvedSubmission() || state.server?.phase === "submitting" ? "Awaiting receipt…" : "Submit to NEAR");
    $("submit-button").disabled = !canSubmit;
    $("discard-button").hidden = !prepared || state.server?.phase !== "signed" || busy() || unresolvedSubmission();
    setText("discard-button", "Edit transfer");
    setText("approval-note", prepared && state.server?.phase === "signed" ? "Signature ready. Submit to execute this exact transfer." : "Fresh capture per approval. Signs this exact transfer.");
}

function renderProof() {
    GnorixProof.render(activeWallet(), displayedProof(), transferFinished());
}

function setPhase(id, status) {
    const node = $(id);
    node.className = status ? `is-${status}` : "";
    if (status === "current") node.setAttribute("aria-current", "step");
    else node.removeAttribute("aria-current");
}

function renderExecution() {
    const server = state.server;
    const prepared = displayedProof();
    const transaction = prepared ? server?.transaction : null;
    let message = "No transfer submitted.";
    let badge = "Ready when you are";
    let kind = "";
    setPhase("phase-sign", "");
    setPhase("phase-submit", "");
    setPhase("phase-receipt", "");
    if (!server) {
        message = "Could not reach the local server. This page will reconnect automatically.";
        badge = "Server unavailable";
        kind = "error";
    } else if (server.phase === "starting") {
        message = server.message;
        badge = "Preparing sandbox";
    } else if (!state.connected) {
        message = "Connection interrupted. Reconnect before retrying; an in-flight transaction may still execute.";
        badge = "Connection interrupted";
        kind = "error";
    } else if (server.phase === "signing") {
        message = server.message;
        badge = "Reconstructing key";
        setPhase("phase-sign", "current");
    } else if (prepared) {
        setPhase("phase-sign", prepared.verified ? "complete" : "failed");
        if (server.phase === "signed") {
            message = "The fingerprint reconstructed an enrolled key. Review the evidence, then submit the signed transfer.";
            badge = prepared.verified ? "Signature ready" : "Signature unverified";
        } else if (server.phase === "submitting" || transaction?.status === "pending") {
            message = server.message;
            badge = "Waiting for receipts";
            setPhase("phase-submit", "current");
            setPhase("phase-receipt", "current");
        } else if (transaction?.status === "succeeded") {
            message = server.message;
            badge = "Previous transfer confirmed";
            kind = "success";
            setPhase("phase-submit", "complete");
            setPhase("phase-receipt", "complete");
        } else if (transaction?.status === "unknown" || server.phase === "unknown") {
            message = server.message || "The outcome is not known yet. Refresh to query the chain before trying another transfer.";
            badge = "Outcome unconfirmed";
            setPhase("phase-submit", "current");
            setPhase("phase-receipt", "current");
        } else if (transaction?.status === "failed" || transaction?.status === "rejected" || server.phase === "failed") {
            message = server.error || server.message;
            badge = transaction?.status === "rejected" ? "Previous request rejected" : "Previous transfer failed";
            kind = "error";
            setPhase("phase-submit", transaction?.status === "rejected" ? "failed" : "complete");
            setPhase("phase-receipt", "failed");
        }
    } else if (server.phase === "failed") {
        message = server.error || server.message;
        badge = activeCapture() ? "Capture failed" : "Request failed";
        kind = "error";
        if (activeCapture()) setPhase("phase-sign", "failed");
    }
    if (prepared && transferFinished()) {
        const recipient = server.wallets.find((wallet) => wallet.id === prepared.recipient);
        message = `${prepared.amount} ${server.symbol} to ${recipient?.name ?? "the signed recipient"}. ${message}`;
    }
    if (transaction?.observation_error) {
        message += ` Balance/status observation: ${transaction.observation_error}`;
    }
    if (transaction?.failures?.length) {
        message += ` ${transaction.failures.map((failure) => `${failure.executor_id}: ${failure.error}`).join(" · ")}`;
    }
    if ($("execution-message").textContent !== message) $("status-content").scrollTop = 0;
    setText("execution-message", message);
    $("execution-message").dataset.kind = kind;
    setText("execution-badge", badge);
    $("transaction-row").hidden = !transaction?.tx_hash;
    setText("transaction-hash", transaction?.tx_hash ?? "");
    const gas = transaction?.gas_burnt;
    $("gas-row").hidden = gas === null || gas === undefined;
    setText("gas-burned", gas === null || gas === undefined ? "" : `${formatUnits(gas, 12)} Tgas`);
}

function render() {
    renderConnection();
    renderWallets();
    renderFingerprint();
    renderForm();
    renderProof();
    renderExecution();
}

async function fetchState() {
    try {
        const response = await fetch("/api/state", { cache: "no-store", headers: { Accept: "application/json" }, signal: AbortSignal.timeout(8000) });
        if (!response.ok) throw new Error(`State request failed (${response.status}).`);
        const snapshot = await response.json();
        const restarted = state.server && state.server.token !== snapshot.token;
        if (restarted) {
            state.dismissedProofs.clear();
            state.imageKey = null;
            state.pointsKey = null;
            state.captureKey = null;
            state.imageRequest += 1;
            state.wallet = null;
            $("amount").value = "";
        }
        state.server = snapshot;
        state.connected = true;
        if (snapshot.wallets.length && (state.wallet === null || !activeWallet())) {
            state.wallet = snapshot.prepared?.wallet ?? snapshot.wallets[0].id;
            if (snapshot.prepared) $("amount").value = snapshot.prepared.amount;
        }
    } catch {
        state.connected = false;
    } finally {
        render();
    }
}

function readState() {
    if (!state.readPromise) {
        state.readPromise = fetchState().finally(() => { state.readPromise = null; });
    }
    return state.readPromise;
}

async function readFreshState() {
    if (state.readPromise) await state.readPromise;
    await readState();
}

async function post(path, body) {
    if (!state.connected || !state.server?.token) throw new Error("Reconnect to the local server first.");
    const response = await fetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json", "X-Gnorix-Token": state.server.token },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(10000),
    });
    if (!response.ok) {
        const failure = await response.json().catch(() => ({}));
        throw new Error(failure.error || `Request rejected (${response.status}).`);
    }
}

async function performRequest(path, body) {
    state.requestInFlight = true;
    showFormError(null);
    render();
    try {
        await post(path, body);
        await readFreshState();
    } catch (error) {
        showFormError(error.message);
        await readFreshState();
    } finally {
        state.requestInFlight = false;
        render();
    }
}

async function signTransfer(event) {
    event.preventDefault();
    if (busy() || !state.connected || unresolvedSubmission()) return;
    try {
        const input = transferInput();
        dismissProof();
        await performRequest("/api/sign", input);
    } catch (error) {
        showFormError(error.message);
    }
}

async function submitTransfer() {
    const prepared = displayedProof();
    if (!prepared?.verified || busy() || state.server?.phase !== "signed") return;
    await performRequest("/api/submit", { prepared_id: prepared.id });
}

async function copyText(button, value, label) {
    if (!value) return;
    try {
        await navigator.clipboard.writeText(value);
        button.textContent = "Copied";
        setTimeout(() => { button.textContent = label; }, 1500);
    } catch {
        showFormError("Clipboard access is unavailable. You can select and copy the displayed text.");
    }
}

async function poll() {
    await readState();
    setTimeout(poll, state.connected ? 500 : 1500);
}

$("transfer-form").addEventListener("submit", signTransfer);
$("amount").addEventListener("input", dismissProof);
$("amount").addEventListener("blur", () => {
    if (!$("amount").value.trim()) return;
    try { transferInput(); } catch (error) { showFormError(error.message); }
});
$("submit-button").addEventListener("click", submitTransfer);
$("discard-button").addEventListener("click", () => {
    dismissProof();
    $("amount").focus();
});
$("refresh-state").addEventListener("click", () => {
    if (!busy()) void performRequest("/api/refresh", {});
});
$("copy-proof").addEventListener("click", () => {
    void copyText($("copy-proof"), GnorixProof.text(), "Copy evidence");
});
$("copy-transaction").addEventListener("click", () => {
    void copyText($("copy-transaction"), state.server?.transaction?.tx_hash, "Copy");
});
new ResizeObserver(renderFingerprint).observe($("fingerprint"));
void poll();
