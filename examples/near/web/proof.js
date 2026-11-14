// Public evidence presentation. Contract submission still uses the original
// Authorization object; this view never changes what is signed or submitted.
const GnorixProof = (() => {
    const encoder = new TextEncoder();
    let activeKey = null;
    let generation = 0;
    let copyValue = "";

    const element = id => document.getElementById(id);
    const setText = (id, value) => { element(id).textContent = value; };
    const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");

    function publicKeyHex(prepared) {
        return prepared ? hex(prepared.authorization.public_key) : "";
    }

    function digestBytes(value) {
        if (!/^[a-f0-9]{64}$/i.test(value ?? "")) throw new Error("Invalid Merkle digest.");
        return Uint8Array.from(value.match(/../g), byte => parseInt(byte, 16));
    }

    function siblingBytes(value) {
        if (!Array.isArray(value) || value.length !== 32 || value.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)) {
            throw new Error("Invalid Merkle sibling.");
        }
        return Uint8Array.from(value);
    }

    function countBytes(value) {
        const bytes = new Uint8Array(8);
        new DataView(bytes.buffer).setBigUint64(0, value, false);
        return bytes;
    }

    async function digest(tag, ...parts) {
        const prefix = encoder.encode(tag);
        const buffer = new Uint8Array(prefix.length + parts.reduce((size, part) => size + part.length, 0));
        buffer.set(prefix);
        let offset = prefix.length;
        for (const part of parts) {
            buffer.set(part, offset);
            offset += part.length;
        }
        return new Uint8Array(await crypto.subtle.digest("SHA-256", buffer));
    }

    // Mirrors the documented core Merkle framing, including duplicate-last
    // nodes and the separate root commitment binding the total leaf count.
    async function tracePath(leaf, authorization, registeredRoot) {
        const count = BigInt(authorization.leaf_count);
        let index = BigInt(authorization.leaf_index);
        if (count < 1n || count > 0xffffffffffffffffn || index < 0n || index >= count) {
            throw new Error("Invalid Merkle position.");
        }
        let width = count;
        let current = digestBytes(leaf);
        const levels = [];
        for (const bytes of authorization.siblings) {
            if (width === 1n) throw new Error("Merkle path has extra siblings.");
            const sibling = siblingBytes(bytes);
            const currentOnLeft = index % 2n === 0n;
            const duplicated = currentOnLeft && index === width - 1n;
            if (duplicated && hex(current) !== hex(sibling)) throw new Error("Invalid duplicate-last sibling.");
            const left = currentOnLeft ? current : sibling;
            const right = currentOnLeft ? sibling : current;
            const parent = await digest("gnorix.merkle.v1.node", left, right);
            levels.push({ left: hex(left), right: hex(right), parent: hex(parent), currentOnLeft, duplicated });
            current = parent;
            index /= 2n;
            width = (width + 1n) / 2n;
        }
        if (width !== 1n) throw new Error("Merkle path is incomplete.");
        const root = hex(await digest("gnorix.merkle.v1.root", countBytes(count), current));
        return { leaf, levels, top: hex(current), root, matches: root === hex(digestBytes(registeredRoot)) };
    }

    function hashNode(label, value, kind = "") {
        const node = document.createElement("div");
        node.className = `merkle-node ${kind}`;
        const title = document.createElement("span");
        title.className = "merkle-node-label";
        title.textContent = label;
        const code = document.createElement("code");
        code.textContent = value;
        node.append(title, code);
        return node;
    }

    function branchLevel(level, index) {
        const row = document.createElement("li");
        row.className = "merkle-level";
        const pair = document.createElement("div");
        pair.className = "merkle-pair";
        const siblingLabel = level.duplicated ? "Duplicate last node" : "Sibling subtree";
        pair.append(
            hashNode(`${level.currentOnLeft ? "Current path" : siblingLabel} · left`, level.left, level.currentOnLeft ? "path-node" : "sibling-node"),
            hashNode(`${level.currentOnLeft ? siblingLabel : "Current path"} · right`, level.right, level.currentOnLeft ? "sibling-node" : "path-node"),
        );
        const parent = hashNode(`SHA-256 → parent ${index + 1}`, level.parent, "parent-node");
        row.append(pair, parent);
        return row;
    }

    function showPath(path, authorization) {
        const nodes = document.createDocumentFragment();
        const leaf = document.createElement("li");
        leaf.className = "merkle-origin";
        leaf.append(hashNode(`Selected leaf ${authorization.leaf_index} · public key + enrollment context`, path.leaf, "path-node"));
        nodes.append(leaf);
        path.levels.forEach((level, index) => nodes.append(branchLevel(level, index)));
        const root = document.createElement("li");
        root.className = "merkle-root-step";
        root.append(hashNode(`Count-bound root · ${authorization.leaf_count} leaves`, path.root, path.matches ? "root-matches" : "root-mismatch"));
        nodes.append(root);
        element("merkle-path").replaceChildren(nodes);
        element("merkle-empty").hidden = true;
        setText("merkle-status", path.matches ? "Root matches" : "Root mismatch");
        element("merkle-status").dataset.kind = path.matches ? "success" : "error";
    }

    function evidenceText(wallet, prepared) {
        if (!prepared) return "";
        const auth = prepared.authorization;
        const lines = [
            "Gnorix transaction evidence",
            `Wallet: ${wallet.account_id}`,
            `Amount: ${prepared.amount} GXT`,
            `Registered Merkle root: ${wallet.root}`,
            `Public key: ${hex(auth.public_key)}`,
            `Signature: ${hex(auth.signature)}`,
            `Nonce: ${auth.action.nonce}`,
            `Selected leaf: ${auth.leaf_index} / ${auth.leaf_count}`,
            `Leaf hash: ${prepared.leaf_hash}`,
            "Merkle siblings (leaf to root):",
        ];
        let index = BigInt(auth.leaf_index);
        auth.siblings.forEach((sibling, depth) => {
            lines.push(`  ${depth + 1}. ${index % 2n === 0n ? "right" : "left"}: ${hex(sibling)}`);
            index /= 2n;
        });
        return lines.join("\n");
    }

    function renderFields(wallet, prepared, previous) {
        const auth = prepared?.authorization;
        setText("proof-root", wallet?.root ?? "—");
        setText("proof-public-key", auth ? publicKeyHex(prepared) : "Available after signing");
        setText("proof-signature", auth ? hex(auth.signature) : "Available after signing");
        setText("proof-nonce", auth?.action.nonce ?? wallet?.nonce ?? "—");
        setText("proof-depth", auth?.siblings.length ?? "—");
        setText("proof-leaf-count", wallet?.leaf_count ?? "—");
        setText("proof-hint", prepared
            ? `${previous ? "Previous transfer · " : ""}${prepared.verified ? "Contract verified" : "Unverified"} · ${prepared.attempts} candidates checked`
            : "Sign a transfer to reveal its membership path");
        element("copy-proof").disabled = !prepared;
        copyValue = evidenceText(wallet, prepared);
    }

    function renderPath(wallet, prepared) {
        const key = prepared ? JSON.stringify([wallet.root, prepared.id, prepared.leaf_hash, prepared.authorization.leaf_index, prepared.authorization.leaf_count, prepared.authorization.siblings]) : null;
        if (key === activeKey) return;
        activeKey = key;
        const request = ++generation;
        element("merkle-path").replaceChildren();
        element("merkle-empty").hidden = false;
        element("merkle-scroll").scrollTop = 0;
        element("merkle-status").dataset.kind = "";
        setText("merkle-status", prepared ? "Rebuilding path…" : "Awaiting signature");
        setText("merkle-empty", prepared ? "Reconstructing the Merkle root from the selected leaf and its sibling hashes…" : "An enrolled key will connect to this root through a Merkle path. Approve a transfer to see the exact nodes.");
        if (!prepared) return;
        void tracePath(prepared.leaf_hash, prepared.authorization, wallet.root).then(path => {
            if (request === generation) showPath(path, prepared.authorization);
        }).catch(() => {
            if (request !== generation) return;
            setText("merkle-status", "Path unavailable");
            element("merkle-status").dataset.kind = "error";
            setText("merkle-empty", "The Merkle path could not be reconstructed for display. The contract verification result is shown separately.");
        });
    }

    function render(wallet, prepared, previous) {
        renderFields(wallet, prepared, previous);
        renderPath(wallet, prepared);
    }

    return { render, tracePath, publicKeyHex, text: () => copyValue };
})();
