"use strict";
// openmod admin interface: one page, no framework, no build step. The server
// embeds this file. Server data is always inserted as text, never as HTML.

const root = document.getElementById("app");
const toastEl = document.getElementById("toast");

let S = null; // the last /api/state document
let poll = null; // download progress timer
const busy = new Set(); // tools being switched
const draft = {}; // unsaved workflow edits, by workflow name
let selected = "default"; // workflow being edited
let creating = false; // the "new workflow" form is open
let step = 0; // setup step
let shownKey = null; // a key just created, shown once
let tryMode = "text";
let tryText = "";
let tryFile = null;
let tryPreview = null;
let tryWorkflow = "default";
let tryResult = null;
let tryRunning = false;

/* ---------- helpers ---------- */

function h(tag, attrs, ...children) {
  const [name, ...classes] = tag.split(".");
  const el = document.createElement(name || "div");
  if (classes.length) el.className = classes.join(" ");
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "style") {
      // Through the CSSOM: the CSP refuses `style` attributes.
      for (const decl of String(v).split(";")) {
        const i = decl.indexOf(":");
        if (i > 0) el.style.setProperty(decl.slice(0, i).trim(), decl.slice(i + 1).trim());
      }
    } else if (k === "value") el.value = v;
    else if (k === "checked" || k === "disabled" || k === "selected") el[k] = Boolean(v);
    else el.setAttribute(k, v === true ? "" : String(v));
  }
  add(el, ...children);
  return el;
}

/** Appends nodes and text, flattening arrays and skipping null and false. */
function add(parent, ...children) {
  for (const c of children.flat(Infinity)) {
    if (c === null || c === undefined || c === false) continue;
    parent.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

class SignedOut extends Error {}

async function api(method, path, body) {
  const headers = { "x-openmod-admin": "1" };
  let payload;
  if (body instanceof Blob) {
    payload = body;
    headers["content-type"] = body.type || "application/octet-stream";
  } else if (body !== undefined) {
    payload = JSON.stringify(body);
    headers["content-type"] = "application/json";
  }
  const res = await fetch(path, { method, headers, body: payload, credentials: "same-origin" });
  let data = null;
  try {
    data = await res.json();
  } catch {
    data = null;
  }
  if (res.status === 401 && path !== "/api/login") {
    S = null;
    stopPoll();
    renderLogin();
    throw new SignedOut("signed out");
  }
  if (!res.ok) throw new Error((data && data.error && data.error.message) || `${res.status} ${res.statusText}`);
  return data;
}

function toast(message, kind) {
  toastEl.textContent = message;
  toastEl.className = `show ${kind || ""}`;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (toastEl.className = ""), kind === "bad" ? 6000 : 2600);
}

function fail(e) {
  if (!(e instanceof SignedOut)) toast(e.message, "bad");
}

function size(mb) {
  if (mb >= 1000) return `${(mb / 1000).toFixed(2).replace(/\.?0+$/, "")} GB`;
  return `${mb} MB`;
}

function day(seconds) {
  return new Date(seconds * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied");
  } catch {
    toast("Copy failed: select the text and copy it by hand", "bad");
  }
}

const toolName = (id) => (S.tools.find((t) => t.id === id) || { name: id }).name;
const category = (kind, id) => S.categories[kind].find((c) => c.id === id);
const keyCount = () => S.keys.length + S.external_keys;

/* ---------- boot, routing ---------- */

const pages = { tools: renderTools, workflows: renderWorkflows, keys: renderKeys, try: renderTry };

function page() {
  const name = (location.hash.match(/^#\/(\w+)/) || [])[1];
  return pages[name] ? name : "tools";
}

async function refresh() {
  S = await api("GET", "/api/state");
  if (!S.workflows[selected]) selected = "default";
  if (!S.workflows[tryWorkflow]) tryWorkflow = "default";
  render();
  watchDownloads();
}

function render() {
  if (!S) return;
  if (!S.setup_done) return renderSetup();
  const current = page();
  const nav = [
    ["tools", "Tools"],
    ["workflows", "Workflows"],
    ["keys", "API keys"],
    ["try", "Try it"],
  ];
  const memory = S.memory.rss_mb
    ? `${size(S.memory.estimate_mb)} estimated, ${size(S.memory.rss_mb)} in use`
    : `${size(S.memory.estimate_mb)} estimated`;
  const main = h("main");
  pages[current](main);
  root.replaceChildren(
    h(
      "header.top",
      null,
      h(
        "div.inner",
        null,
        h("div.brand", null, "openmod"),
        h(
          "nav",
          { "aria-label": "Sections" },
          nav.map(([id, label]) => h("a", { href: `#/${id}`, "aria-current": id === current ? "page" : null }, label)),
        ),
        h("span.pill", { title: "Memory the tools that are on need" }, memory),
        h("button.ghost", { onclick: signOut }, "Sign out"),
      ),
    ),
    main,
  );
}

window.addEventListener("hashchange", () => {
  if (S) render();
});

async function signOut() {
  try {
    await api("POST", "/api/logout");
  } catch (e) {
    fail(e);
  }
  S = null;
  stopPoll();
  renderLogin();
}

/* ---------- sign in ---------- */

function renderLogin(error) {
  const input = h("input", {
    type: "password",
    id: "token",
    autocomplete: "off",
    spellcheck: "false",
    placeholder: "oma_...",
  });
  const form = h(
    "form.card.login",
    {
      onsubmit: async (e) => {
        e.preventDefault();
        const button = form.querySelector("button");
        button.disabled = true;
        try {
          await api("POST", "/api/login", { token: input.value.trim() });
          await refresh();
        } catch (err) {
          renderLogin(err.message);
        }
      },
    },
    h("div.brand", null, "openmod"),
    h("p.muted", { style: "margin-bottom:22px" }, "Sign in with the admin token printed in the server log at first start."),
    h("label", { for: "token" }, "Admin token"),
    input,
    error ? h("p.error", null, error) : null,
    h("div.actions", null, h("button.primary", { type: "submit" }, "Sign in")),
    h("p.muted.small", { style: "margin-top:22px" }, "Lost it? Run ", h("code", null, "openmod admin-token"), " on the server for a new one."),
  );
  root.replaceChildren(h("div.center", null, form));
  input.focus();
}

/* ---------- tools ---------- */

const STATUS = { ready: "Ready", disabled: "Off", absent: "Off", missing: "Files missing", failed: "Failed to load" };

async function setTool(id, enabled) {
  busy.add(id);
  render();
  try {
    S = await api("POST", `/api/tools/${encodeURIComponent(id)}`, { enabled });
    toast(enabled ? `${toolName(id)} is on` : `${toolName(id)} is off`);
  } catch (e) {
    fail(e);
  } finally {
    busy.delete(id);
    render();
  }
}

function lostRules(tool) {
  return tool.needed_by.map((u) => u.split(".")).map(([wf, , cat]) => `${wf} (${cat})`);
}

function toolRow(tool) {
  const working = busy.has(tool.id);
  const state = working ? "busy" : tool.status;
  const notes = [];
  if (tool.role === "second_opinion") {
    notes.push(h("p.note.muted", null, "Runs as a second opinion, only on the texts GLiGuard finds risky."));
  }
  if (tool.status === "failed") notes.push(h("p.note.bad", null, tool.error));
  if (tool.status === "missing" && !tool.downloadable) {
    notes.push(h("p.note.warn", null, `Missing: ${tool.missing.join(", ")}. Put these files next to the others.`));
  }
  const lost = lostRules(tool);
  if (!tool.enabled && lost.length) {
    notes.push(h("p.note.warn", null, `Not checked while off: ${lost.join(", ")}`));
  }
  const canDownload = tool.status === "missing" && tool.downloadable && !S.downloads.running;
  return h(
    "div.row",
    null,
    h(
      "label.switch",
      null,
      h("input", {
        type: "checkbox",
        checked: tool.enabled,
        disabled: working,
        "aria-label": `${tool.name}: ${tool.enabled ? "on" : "off"}`,
        onchange: (e) => setTool(tool.id, e.target.checked),
      }),
      h("span"),
    ),
    h(
      "div.body",
      null,
      h("div.title", null, h("h3", null, tool.name), tool.license ? h("span.muted.small", null, tool.license) : null),
      h("p.summary", null, tool.summary),
      tool.categories.length ? h("div.chips", null, tool.categories.map((c) => h("span", null, c))) : null,
      notes,
    ),
    h(
      "div.side",
      null,
      h("span.muted.small", null, size(tool.ram_mb)),
      h("span", { class: `status ${state}` }, working ? "Working..." : STATUS[tool.status] || tool.status),
      canDownload ? h("button", { onclick: () => download([tool.id]) }, `Download ${size(tool.download_mb)}`) : null,
    ),
  );
}

function rulesRow() {
  return h(
    "div.row",
    null,
    h("label.switch", null, h("input", { type: "checkbox", checked: true, disabled: true, "aria-label": "Text rules: always on" }), h("span")),
    h(
      "div.body",
      null,
      h("div.title", null, h("h3", null, "Text rules")),
      h("p.summary", null, "Emails, phone numbers, IBANs, card numbers, links, messaging handoffs, spam, scams, swear words and extremist slogans. Exact matches, in microseconds."),
    ),
    h("div.side", null, h("span.muted.small", null, "no memory"), h("span.status.ready", null, "Always on")),
  );
}

function toolGroups() {
  return [
    h("h2", null, "Images"),
    h("div.card", null, S.tools.filter((t) => t.kind === "image").map(toolRow)),
    h("h2", null, "Texts"),
    h("div.card", null, rulesRow(), S.tools.filter((t) => t.kind === "text").map(toolRow)),
    h(
      "div.total",
      null,
      h("span", null, "Memory with these tools: ", h("strong", null, size(S.memory.estimate_mb))),
      S.memory.rss_mb ? h("span", null, `In use now: ${size(S.memory.rss_mb)}`) : null,
    ),
  ];
}

function renderTools(main) {
  add(main, 
    h("h1", null, "Tools"),
    h("p.lead", null, "Choose what this server loads. A tool that is off uses no memory, and the workflow rules that need it are reported as not checked."),
    progressPanel(),
    toolGroups(),
  );
}

/* ---------- downloads ---------- */

async function download(tools) {
  try {
    S = await api("POST", "/api/downloads", tools ? { tools } : {});
    render();
    watchDownloads();
  } catch (e) {
    fail(e);
  }
}

function missingDownloads() {
  return S.tools.filter((t) => t.enabled && t.status === "missing" && t.downloadable);
}

function progressPanel() {
  const d = S.downloads;
  if (!d.files.length) return h("div", { id: "downloads", hidden: true });
  const done = d.files.filter((f) => f.state === "done").length;
  return h(
    "div.card.progress",
    { id: "downloads", style: "margin:22px 0 4px" },
    h(
      "div.split",
      null,
      h("strong", null, d.running ? "Downloading models" : "Downloads finished"),
      h("span.muted.small", null, `${done} of ${d.files.length} files`),
    ),
    d.files.map((f) => {
      const pct = f.size ? Math.floor((f.done * 100) / f.size) : 0;
      const label = { queued: "Waiting", downloading: `${pct}%`, done: "Done", failed: "Failed" }[f.state];
      return h(
        "div.file",
        null,
        h("span", null, f.name, h("span.muted", null, `  ${size(Math.ceil(f.size / 1e6))}`)),
        h("span", { class: f.state === "failed" ? "bad" : "muted" }, label),
        h("div", { class: `bar ${f.state === "failed" ? "failed" : ""}` }, h("i", { style: `width:${f.state === "done" ? 100 : pct}%` })),
        f.error ? h("span.bad.small", { style: "grid-column:1/-1" }, f.error) : null,
      );
    }),
  );
}

function stopPoll() {
  clearInterval(poll);
  poll = null;
}

function watchDownloads() {
  if (!S || !S.downloads.running || poll) return;
  poll = setInterval(async () => {
    try {
      S.downloads = await api("GET", "/api/downloads");
      const panel = document.getElementById("downloads");
      if (panel) panel.replaceWith(progressPanel());
      if (!S.downloads.running) {
        stopPoll();
        const failed = S.downloads.files.some((f) => f.state === "failed");
        toast(failed ? "Some downloads failed" : "Models downloaded and loaded", failed ? "bad" : "");
        await refresh();
      }
    } catch (e) {
      stopPoll();
      fail(e);
    }
  }, 800);
}

/* ---------- workflows ---------- */

function workflowDraft(name) {
  if (!draft[name]) {
    const wf = S.workflows[name] || {};
    draft[name] = { image: structuredClone(wf.image || {}), text: structuredClone(wf.text || {}) };
  }
  return draft[name];
}

function canonical(wf) {
  const sort = (rules) =>
    Object.keys(rules || {})
      .sort()
      .map((c) => [c, rules[c].reject ?? null, rules[c].review ?? null]);
  return JSON.stringify([sort(wf && wf.image), sort(wf && wf.text)]);
}

function isDirty(name) {
  return Boolean(draft[name]) && canonical(draft[name]) !== canonical(S.workflows[name]);
}

function updateSaveBar() {
  const bar = document.getElementById("savebar");
  if (bar) bar.hidden = !isDirty(selected);
}

function thresholdInput(label, value, onchange) {
  return h(
    "label",
    null,
    label,
    h("input", {
      type: "number",
      min: 0,
      max: 1,
      step: 0.05,
      inputmode: "decimal",
      placeholder: "off",
      value: value ?? "",
      oninput: (e) => {
        onchange(e.target.value === "" ? undefined : Number(e.target.value));
        updateSaveBar();
      },
    }),
  );
}

function ruleRow(name, kind, id) {
  const rules = workflowDraft(name)[kind];
  const t = rules[id];
  const info = category(kind, id) || { title: id, tools: [], rule: false, available: false };
  let controls;
  if (info.rule) {
    controls = h(
      "select",
      {
        "aria-label": `Action for ${info.title}`,
        onchange: (e) => {
          rules[id] = e.target.value === "reject" ? { reject: 1 } : { review: 1 };
          updateSaveBar();
        },
      },
      h("option", { value: "review", selected: t.reject === undefined }, "Flag for review"),
      h("option", { value: "reject", selected: t.reject !== undefined }, "Reject"),
    );
  } else {
    controls = [
      thresholdInput("Review at", t.review, (v) => (t.review = v)),
      thresholdInput("Reject at", t.reject, (v) => (t.reject = v)),
    ];
  }
  const off = !info.available;
  return h(
    "div",
    { class: `rule ${off ? "off" : ""}` },
    h(
      "div.name",
      null,
      info.title,
      h("small", null, id),
      off ? h("span.warn.small", null, `Not checked: turn on ${info.tools.map(toolName).join(" or ") || "a model that scores it"}`) : null,
    ),
    h("div.controls", null, controls),
    h(
      "button.ghost",
      {
        "aria-label": `Remove ${info.title}`,
        onclick: () => {
          delete rules[id];
          render();
        },
      },
      "Remove",
    ),
  );
}

function addRuleSelect(name, kind) {
  const rules = workflowDraft(name)[kind];
  const options = S.categories[kind].filter((c) => !(c.id in rules));
  if (!options.length) return null;
  return h(
    "select.add",
    {
      "aria-label": `Add a ${kind} category`,
      onchange: (e) => {
        const info = category(kind, e.target.value);
        if (!info) return;
        rules[info.id] = info.rule ? { review: 1 } : { reject: 0.8, review: 0.5 };
        render();
      },
    },
    h("option", { value: "" }, "Add a category..."),
    options.map((c) => h("option", { value: c.id }, c.available ? c.title : `${c.title} (its tool is off)`)),
  );
}

function section(name, kind, title) {
  const rules = workflowDraft(name)[kind];
  const ids = Object.keys(rules).sort((a, b) => (category(kind, a)?.title || a).localeCompare(category(kind, b)?.title || b));
  return [
    h("h2", null, title),
    h("div.card", null, ids.length ? ids.map((id) => ruleRow(name, kind, id)) : h("p.empty", null, `No ${kind} rule: this workflow does not check ${kind === "image" ? "images" : "texts"}.`)),
    addRuleSelect(name, kind),
  ];
}

function cleaned(rules) {
  const out = {};
  for (const [id, t] of Object.entries(rules)) {
    const keep = {};
    for (const k of ["reject", "review"]) {
      if (typeof t[k] === "number" && !Number.isNaN(t[k])) keep[k] = t[k];
    }
    if (keep.reject === undefined && keep.review === undefined) throw new Error(`${id}: give a review or a reject threshold, or remove it`);
    out[id] = keep;
  }
  return out;
}

async function saveWorkflow(name) {
  try {
    const d = workflowDraft(name);
    S = await api("PUT", `/api/workflows/${encodeURIComponent(name)}`, { image: cleaned(d.image), text: cleaned(d.text) });
    delete draft[name];
    toast("Saved and applied");
    render();
  } catch (e) {
    fail(e);
  }
}

async function deleteWorkflow(name) {
  if (!confirm(`Delete the workflow "${name}"? Apps that still call it will get an error.`)) return;
  try {
    S = await api("DELETE", `/api/workflows/${encodeURIComponent(name)}`);
    delete draft[name];
    selected = "default";
    toast("Deleted");
    render();
  } catch (e) {
    fail(e);
  }
}

function newWorkflowForm() {
  const input = h("input", { type: "text", placeholder: "avatars", "aria-label": "Workflow name", spellcheck: "false" });
  const form = h(
    "form.split",
    {
      style: "margin-top:4px",
      onsubmit: async (e) => {
        e.preventDefault();
        const name = input.value.trim();
        if (!/^[a-z0-9][a-z0-9_-]{0,39}$/.test(name)) return toast("Use a-z, 0-9, _ and -, up to 40 characters", "bad");
        if (S.workflows[name]) return toast("That workflow already exists", "bad");
        const source = selected;
        const from = workflowDraft(source);
        try {
          S = await api("PUT", `/api/workflows/${encodeURIComponent(name)}`, { image: cleaned(from.image), text: cleaned(from.text) });
          creating = false;
          selected = name;
          toast(`Created "${name}", a copy of "${source}"`);
          render();
        } catch (err) {
          fail(err);
        }
      },
    },
    input,
    h("button.primary", { type: "submit" }, `Copy "${selected}"`),
    h(
      "button.ghost",
      {
        type: "button",
        onclick: () => {
          creating = false;
          render();
        },
      },
      "Cancel",
    ),
  );
  setTimeout(() => input.focus());
  return form;
}

function renderWorkflows(main) {
  const names = Object.keys(S.workflows).sort((a, b) => (a === "default" ? -1 : b === "default" ? 1 : a.localeCompare(b)));
  if (!S.workflows[selected]) selected = "default";
  add(main, 
    h("h1", null, "Workflows"),
    h(
      "p.lead",
      null,
      "A workflow lists what to check and when to act. Send its name with each request: ",
      h("code", null, `?workflow=${selected}`),
      ". A score at or above a threshold flags the content.",
    ),
    h(
      "div.tabs",
      null,
      names.map((n) =>
        h(
          "button",
          {
            "aria-pressed": n === selected ? "true" : "false",
            onclick: () => {
              selected = n;
              creating = false;
              render();
            },
          },
          n,
          isDirty(n) ? " *" : "",
        ),
      ),
      creating
        ? null
        : h(
            "button.ghost",
            {
              onclick: () => {
                creating = true;
                render();
              },
            },
            "New workflow",
          ),
    ),
    creating ? newWorkflowForm() : null,
    section(selected, "image", "Images"),
    section(selected, "text", "Texts"),
    selected === "default"
      ? h("p.muted.small", { style: "margin-top:26px" }, "The default workflow is used when a request names none. It cannot be deleted.")
      : h("div.actions", null, h("button.danger", { onclick: () => deleteWorkflow(selected) }, "Delete this workflow")),
    h(
      "div.savebar",
      { id: "savebar", hidden: !isDirty(selected) },
      h("span", null, "Unsaved changes"),
      h(
        "button.ghost",
        {
          onclick: () => {
            delete draft[selected];
            render();
          },
        },
        "Discard",
      ),
      h("button.primary", { onclick: () => saveWorkflow(selected) }, "Save"),
    ),
  );
}

/* ---------- API keys ---------- */

async function createKey(name) {
  try {
    const res = await api("POST", "/api/keys", { name });
    shownKey = { name, key: res.key };
    S = res.state;
    render();
  } catch (e) {
    fail(e);
  }
}

function keyCreator(placeholder) {
  const input = h("input", { type: "text", placeholder, "aria-label": "Key name", maxlength: 60 });
  return h(
    "form.split",
    {
      onsubmit: (e) => {
        e.preventDefault();
        createKey(input.value.trim() || placeholder);
      },
    },
    input,
    h("button.primary", { type: "submit" }, "Create a key"),
  );
}

function secretBox(key) {
  return h(
    "div.stack",
    null,
    h("div.secret", null, h("code", null, key), h("button", { onclick: () => copy(key) }, "Copy")),
    h("p.warn.small", null, "Shown once. Store it in your app's secrets now: openmod keeps only its digest."),
  );
}

async function revokeKey(key) {
  if (!confirm(`Revoke "${key.name}"? Requests using it will be refused at once.`)) return;
  try {
    S = await api("DELETE", `/api/keys/${encodeURIComponent(key.id)}`);
    toast("Revoked");
    render();
  } catch (e) {
    fail(e);
  }
}

function renderKeys(main) {
  add(main, 
    h("h1", null, "API keys"),
    h("p.lead", null, "Your apps send a key with each request, as ", h("code", null, "Authorization: Bearer <key>"), " or SightEngine's ", h("code", null, "api_secret"), "."),
    h("div.card.pad", { style: "margin-top:22px" }, h("div.stack", null, keyCreator("my-app"), shownKey ? h("p.small", null, `New key "${shownKey.name}":`) : null, shownKey ? secretBox(shownKey.key) : null)),
    h("h2", null, "Keys"),
    h(
      "div.card",
      null,
      S.keys.length
        ? S.keys.map((k) =>
            h(
              "div.key",
              null,
              h("div", null, h("strong", null, k.name), h("div.muted.small.mono", null, `${k.prefix}...`)),
              h("span.muted.small", null, `Created ${day(k.created)}`),
              h("button.ghost.danger", { onclick: () => revokeKey(k) }, "Revoke"),
            ),
          )
        : h("p.empty", null, "No key created here yet."),
    ),
    S.external_keys
      ? h("p.muted.small", { style: "margin-top:14px" }, `Plus ${S.external_keys} key${S.external_keys > 1 ? "s" : ""} from OPENMOD_API_KEYS or api_keys_file, managed outside this interface.`)
      : null,
  );
}

/* ---------- try it ---------- */

async function runTry() {
  if (tryRunning) return;
  if (tryMode === "text" && !tryText.trim()) return toast("Type a text first", "bad");
  if (tryMode === "image" && !tryFile) return toast("Choose an image first", "bad");
  tryRunning = true;
  render();
  try {
    tryResult =
      tryMode === "text"
        ? await api("POST", "/api/check/text", { text: tryText, workflow: tryWorkflow })
        : await api("POST", `/api/check/image?workflow=${encodeURIComponent(tryWorkflow)}`, tryFile);
  } catch (e) {
    tryResult = null;
    fail(e);
  } finally {
    tryRunning = false;
    render();
  }
}

function pickImage(file) {
  if (!file) return;
  tryFile = file;
  if (tryPreview) URL.revokeObjectURL(tryPreview);
  tryPreview = URL.createObjectURL(file);
  tryResult = null;
  render();
}

function imageInput() {
  const input = h("input", { type: "file", accept: "image/*", hidden: true, onchange: (e) => pickImage(e.target.files[0]) });
  const zone = h(
    "div.drop",
    {
      role: "button",
      tabindex: 0,
      onclick: () => input.click(),
      onkeydown: (e) => {
        if (e.key === "Enter" || e.key === " ") input.click();
      },
      ondragover: (e) => {
        e.preventDefault();
        zone.classList.add("over");
      },
      ondragleave: () => zone.classList.remove("over"),
      ondrop: (e) => {
        e.preventDefault();
        pickImage(e.dataTransfer.files[0]);
      },
    },
    tryPreview ? h("img", { src: tryPreview, alt: "The image to check" }) : null,
    tryFile ? `${tryFile.name}, click to change` : "Drop an image here, or click to choose one",
    input,
  );
  return zone;
}

function resultCard(r) {
  const s = r.summary;
  const verdict = { accept: "Accepted", review: "Needs review", reject: "Rejected" }[s.action];
  const reasons = [...s.reject_reason, ...s.review_reason];
  const scores = Object.entries(r.scores).sort((a, b) => b[1] - a[1]);
  return h(
    "div.card.pad",
    { style: "margin-top:22px" },
    h("div", { class: `verdict ${s.action}` }, verdict, h("span.muted.small", null, `${r.time_ms} ms`)),
    reasons.length
      ? h(
          "div",
          { style: "margin-top:14px" },
          reasons.map((x) =>
            h(
              "div.reason",
              null,
              h("strong", null, x.text),
              h("span.muted.small", null, `  ${x.id}, score ${x.score.toFixed(2)}, threshold ${x.threshold.toFixed(2)}, ${x.source}`),
              h("p.muted", null, x.message),
            ),
          ),
        )
      : h("p.muted", { style: "margin-top:8px" }, "Nothing in this workflow matched."),
    scores.length ? h("h2", null, "Scores") : null,
    scores.map(([id, v]) =>
      h("div.score", null, h("span", null, category(tryMode, id)?.title || id), h("div.bar", null, h("i", { style: `width:${Math.round(v * 100)}%` })), h("span.muted", null, v.toFixed(2))),
    ),
    r.unchecked && r.unchecked.length ? h("p.warn.small", { style: "margin-top:16px" }, `Not checked on this server: ${r.unchecked.join(", ")}`) : null,
    h("details", null, h("summary", null, "Response JSON"), h("pre", null, JSON.stringify(r, null, 2))),
  );
}

function renderTry(main) {
  const names = Object.keys(S.workflows).sort();
  const mode = (m, label) =>
    h(
      "button",
      {
        "aria-pressed": tryMode === m ? "true" : "false",
        onclick: () => {
          tryMode = m;
          tryResult = null;
          render();
        },
      },
      label,
    );
  const textArea = h("textarea", {
    placeholder: "Type or paste a text in any language",
    "aria-label": "Text to check",
    value: tryText,
    oninput: (e) => (tryText = e.target.value),
    onkeydown: (e) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) runTry();
    },
  });
  add(main, 
    h("h1", null, "Try it"),
    h("p.lead", null, "Check a text or an image with any workflow, exactly as the API would."),
    h(
      "div.split",
      { style: "margin:22px 0 14px" },
      h("div.seg", { role: "group", "aria-label": "Content type" }, mode("text", "Text"), mode("image", "Image")),
      h("span.spacer"),
      h("label", { style: "margin:0", for: "try-workflow" }, "Workflow"),
      h(
        "select",
        { id: "try-workflow", onchange: (e) => (tryWorkflow = e.target.value) },
        names.map((n) => h("option", { value: n, selected: n === tryWorkflow }, n)),
      ),
    ),
    tryMode === "text" ? textArea : imageInput(),
    h(
      "div.actions",
      null,
      h("button.primary", { onclick: runTry, disabled: tryRunning }, tryRunning ? "Checking..." : "Check"),
      tryMode === "text" ? h("span.muted.small", null, "Ctrl + Enter") : null,
    ),
    tryResult ? resultCard(tryResult) : null,
  );
}

/* ---------- first-run setup ---------- */

async function finishSetup() {
  try {
    S = await api("POST", "/api/setup");
    location.hash = "#/tools";
    render();
  } catch (e) {
    fail(e);
  }
}

function setupTools() {
  return [
    h("h1", null, "What should this server check?"),
    h("p.lead", null, "Turn on what you need. A tool you leave off is not loaded and uses no memory. You can change this at any time."),
    toolGroups(),
    h("div.actions", null, h("span.spacer"), h("button.primary", { onclick: () => go(1) }, "Next")),
  ];
}

function setupModels() {
  const missing = missingDownloads();
  const total = missing.reduce((n, t) => n + t.download_mb, 0);
  const manual = S.tools.filter((t) => t.enabled && t.status === "missing" && !t.downloadable);
  const running = S.downloads.running;
  let body;
  if (running || S.downloads.files.length) body = progressPanel();
  else if (missing.length) {
    body = h(
      "div.stack",
      null,
      h("p", null, `${missing.map((t) => t.name).join(", ")}: ${size(total)} to download. Models are checked against pinned SHA-256 digests; domain lists come from their publishers and are refreshed every day.`),
      h("div", null, h("button.primary", { onclick: () => download() }, `Download ${size(total)}`)),
    );
  } else body = h("p", null, "Every model file is here.");
  return [
    h("h1", null, "Download the models"),
    h("p.lead", null, "openmod downloads each model once, then runs it on this machine. Nothing you check ever leaves it."),
    h("div", { style: "margin-top:22px" }, body),
    manual.length ? h("p.warn.small", { style: "margin-top:14px" }, `${manual.map((t) => t.name).join(", ")}: put the missing files in place by hand.`) : null,
    h("div.actions", null, h("button.ghost", { onclick: () => go(0) }, "Back"), h("span.spacer"), h("button.primary", { onclick: () => go(2) }, running ? "Continue while it downloads" : "Next")),
  ];
}

function setupKey() {
  return [
    h("h1", null, "Create an API key"),
    h("p.lead", null, "Your app sends this key with every request. Create one per app, so you can revoke one without touching the others."),
    h("div", { style: "margin-top:22px" }, shownKey ? secretBox(shownKey.key) : keyCreator("my-app")),
    !shownKey && keyCount() ? h("p.muted.small", { style: "margin-top:12px" }, `This server already has ${keyCount()} key${keyCount() > 1 ? "s" : ""}.`) : null,
    h("div.actions", null, h("button.ghost", { onclick: () => go(1) }, "Back"), h("span.spacer"), h("button.primary", { onclick: () => go(3), disabled: !keyCount() }, "Next")),
  ];
}

function setupDone() {
  const example = `curl -H "Authorization: Bearer $OPENMOD_KEY" \\\n  --data-binary @photo.jpg \\\n  "${S.api_url}/v1/image?workflow=default"`;
  return [
    h("h1", null, "You are set"),
    h("p.lead", null, "openmod is ready. Send it an image or a text; the answer says accept, review or reject, with the reasons."),
    h("div.stack", { style: "margin-top:22px" }, h("pre", null, example), h("div", null, h("button", { onclick: () => copy(example) }, "Copy"))),
    h("div.actions", null, h("button.ghost", { onclick: () => go(2) }, "Back"), h("span.spacer"), h("button.primary", { onclick: finishSetup }, "Open the dashboard")),
  ];
}

function go(n) {
  step = n;
  render();
  window.scrollTo(0, 0);
}

function renderSetup() {
  const steps = ["Tools", "Models", "API key", "Done"];
  const content = [setupTools, setupModels, setupKey, setupDone][step]();
  root.replaceChildren(
    h(
      "div.setup",
      null,
      h("div.setup-head", null, h("div.brand", null, "openmod"), h("button.ghost", { onclick: finishSetup }, "Skip setup")),
      h(
        "ol.steps",
        { "aria-label": "Setup steps" },
        steps.map((s, i) => h("li", { class: i === step ? "on" : i < step ? "done" : "", "aria-current": i === step ? "step" : null }, h("span", null, i + 1), s)),
      ),
      h("div.card.pad", null, content),
    ),
  );
}

/* ---------- start ---------- */

(async () => {
  try {
    const session = await api("GET", "/api/session");
    if (!session.authenticated) return renderLogin();
    await refresh();
  } catch (e) {
    fail(e);
  }
})();
