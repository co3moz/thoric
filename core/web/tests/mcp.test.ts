import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"

import {
  defaultAlias,
  endpointApi,
  endpointUrl,
  excludedTools,
  grantDraft,
  grantRequests,
  imageServers,
  inProfile,
  isServerTool,
  localUrl,
  newGrant,
  orderScopes,
  parseEnv,
  parseHeaders,
  parseLines,
  profileDescription,
  profileLabel,
  PROFILES,
  readServerDeclarations,
  scopeSummary,
  serverApi,
  serverCommand,
  serverDraft,
  serverRequest,
  serversByContainer,
  serversSummary,
  serverToolCount,
  SCOPES,
  thoricTools,
  toolGroups,
} from "../src/lib/mcp"
import type { DesktopServer, EndpointTool } from "../src/lib/types"

test("a container's default name at an endpoint is its name in lowercase letters and dashes", () => {
  assert.equal(defaultAlias("Unity Workspace"), "unity-workspace")
  assert.equal(defaultAlias("  Işık / Çalışma #2 "), "isik-calisma-2")
  assert.equal(defaultAlias("---"), "desktop")
  // Another container of the endpoint already goes by it, so a number is added.
  assert.equal(defaultAlias("Unity", ["unity"]), "unity-2")
  assert.equal(defaultAlias("Unity", ["unity", "unity-2"]), "unity-3")
  // At most 24 characters, as the manager allows.
  const long = defaultAlias("a".repeat(40))
  assert.equal(long, "a".repeat(24))
  assert.equal(defaultAlias("a".repeat(40), [long]), `${"a".repeat(22)}-2`)
  // A cut that ends on a dash leaves the dash out.
  assert.equal(defaultAlias(`${"a".repeat(23)} b`), "a".repeat(23))
})

test("grants are sent with their scopes in order and the servers they include", () => {
  const draft = newGrant({ id: "c1", name: "Unity" }, ["input", "view"], [])
  assert.deepEqual(draft, {
    container_id: "c1",
    container_name: "Unity",
    alias: "unity",
    scopes: ["view", "input"],
    profile: "full",
    all: true,
    servers: [],
    excluded: [],
  })
  assert.deepEqual(
    grantRequests([
      draft,
      { ...draft, container_id: "c2", alias: "  ", all: false, servers: ["chrome"] },
    ]),
    [
      { container_id: "c1", alias: "unity", scopes: ["view", "input"], servers: "all" },
      // An empty name is left to the manager to choose.
      { container_id: "c2", scopes: ["view", "input"], servers: ["chrome"] },
    ]
  )
  assert.throws(() => grantRequests([]), /at least one container/)
  assert.throws(
    () => grantRequests([draft, { ...draft, container_id: "c2", alias: " unity " }]),
    /Two containers go by unity/
  )
  // An existing grant comes back into the form as it was, a chosen list included.
  assert.deepEqual(
    grantDraft({
      container_id: "c1",
      container_name: "Unity",
      alias: "u",
      scopes: ["power", "view"],
      servers: ["x"],
      excluded_tools: ["container_action"],
    }),
    {
      container_id: "c1",
      container_name: "Unity",
      alias: "u",
      scopes: ["view", "power"],
      // A grant from before profiles, or from a manager without them, offers every tool.
      profile: "full",
      all: false,
      servers: ["x"],
      excluded: ["container_action"],
    }
  )
})

test("a grant's profile is sent unless it is full, and comes back into the form", () => {
  const draft = newGrant({ id: "c1", name: "Unity" }, ["view"], [])
  assert.ok(!("profile" in grantRequests([draft])[0]))
  assert.equal(grantRequests([{ ...draft, profile: "headless" }])[0].profile, "headless")
  assert.equal(
    grantDraft({
      container_id: "c1",
      container_name: "Unity",
      alias: "u",
      scopes: ["view"],
      profile: "desktop",
      servers: "all",
      excluded_tools: [],
    }).profile,
    "desktop"
  )
  // The forms list every profile the catalog names, `full` first, as the manager's default is.
  assert.deepEqual(PROFILES, ["full", "headless", "desktop"])
  for (const profile of PROFILES) {
    assert.ok(profileLabel(profile) && profileLabel(profile) !== profile)
    assert.ok(profileDescription(profile))
  }
  // One a newer manager added is shown as it calls it.
  assert.equal(profileLabel("kiosk"), "kiosk")
  assert.equal(profileDescription("kiosk"), "")
})

test("the tools a grant offers are counted by the manager's lists", () => {
  const scopes = {
    scopes: [
      { id: "view" as const, tools: ["get_container", "screenshot", "list_processes"] },
      { id: "commands" as const, tools: ["exec_container", "open_terminal"] },
      { id: "scale" as const, tools: ["create_instance"] },
    ],
    default: ["view" as const],
    set_tools: ["list_instances"],
    job_scopes: ["commands" as const, "scale" as const],
    profiles: [
      {
        id: "full" as const,
        tools: [
          "get_container",
          "screenshot",
          "list_processes",
          "exec_container",
          "open_terminal",
          "create_instance",
          "list_instances",
          "get_job",
        ],
      },
      {
        id: "headless" as const,
        tools: [
          "get_container",
          "list_processes",
          "exec_container",
          "open_terminal",
          "create_instance",
          "list_instances",
          "get_job",
        ],
      },
      {
        id: "desktop" as const,
        tools: [
          "get_container",
          "screenshot",
          "exec_container",
          "create_instance",
          "list_instances",
          "get_job",
        ],
      },
    ],
  }
  const draft = newGrant({ id: "c1", name: "Unity" }, ["view", "commands"], [])
  // `get_job` comes with a scope whose tools return a job.
  assert.deepEqual(thoricTools(draft, scopes, "full"), [
    "get_container",
    "screenshot",
    "list_processes",
    "exec_container",
    "open_terminal",
    "get_job",
  ])
  assert.deepEqual(thoricTools(draft, scopes, "headless"), [
    "get_container",
    "list_processes",
    "exec_container",
    "open_terminal",
    "get_job",
  ])
  assert.deepEqual(thoricTools(draft, scopes, "desktop"), [
    "get_container",
    "screenshot",
    "exec_container",
    "get_job",
  ])
  assert.deepEqual(thoricTools({ ...draft, scopes: ["view"] }, scopes, "full"), [
    "get_container",
    "screenshot",
    "list_processes",
  ])
  // A tool left out by name is not counted, in whatever profile.
  assert.deepEqual(
    thoricTools({ ...draft, excluded: ["get_job", "get_container"] }, scopes, "desktop"),
    ["screenshot", "exec_container"]
  )
  // A scale set's endpoint always has its own tools, and `scale` brings jobs too.
  assert.deepEqual(thoricTools({ ...draft, set: "re", scopes: ["scale"] }, scopes, "headless"), [
    "list_instances",
    "create_instance",
    "get_job",
  ])
  // Without the manager's lists nothing is counted, and without its profiles nothing is narrowed.
  assert.deepEqual(thoricTools(draft, undefined, "headless"), [])
  assert.ok(inProfile("headless", "screenshot"))
  assert.ok(inProfile("headless", "screenshot", { scopes: [], default: [] }))
  assert.ok(!inProfile("headless", "screenshot", scopes))
  assert.ok(inProfile("full", "a_tool_of_tomorrow", scopes))
  assert.ok(!inProfile("desktop", "a_tool_of_tomorrow", scopes))

  // Desktop servers' tools: those of the servers included, without the ones left out.
  const servers = [
    {
      name: "ghidra",
      tools: [
        { name: "decompile", exposed: "ghidra__decompile", description: "" },
        { name: "rename", exposed: "ghidra__rename", description: "" },
      ],
    },
    { name: "chrome", tools: [{ name: "click", exposed: "chrome__click", description: "" }] },
  ]
  assert.equal(serverToolCount(draft, servers), 3)
  assert.equal(serverToolCount({ ...draft, all: false, servers: ["chrome"] }, servers), 1)
  assert.equal(serverToolCount({ ...draft, excluded: ["ghidra__rename"] }, servers), 2)
  assert.equal(serverToolCount({ ...draft, all: false, servers: [] }, servers), 0)
})

// The manager's own list is `Profile::ALL` in core/manager/src/desktop_mcp/profiles.rs, whose
// tests hold it to PROFILES and the catalog; from here, a profile the manager no longer has.
test("every profile the forms list is one the manager has", () => {
  const source = readFileSync(
    new URL("../../manager/src/desktop_mcp/profiles.rs", import.meta.url),
    "utf8"
  )
  for (const profile of PROFILES)
    assert.ok(
      source.includes(`=> "${profile}",`),
      `core/manager/src/desktop_mcp/profiles.rs has no profile ${profile}`
    )
})

test("tools a grant leaves out are sent once each, container tools only with their scope", () => {
  const scopes = {
    scopes: [
      { id: "view" as const, tools: ["screenshot", "list_windows"] },
      { id: "applications" as const, tools: ["application_diagnostics"] },
    ],
    default: ["view" as const],
  }
  const draft = {
    ...newGrant({ id: "c1", name: "Unity" }, ["view"], []),
    excluded: [
      "application_diagnostics",
      "screenshot",
      "playwright__browser_run_code_unsafe",
      "screenshot",
    ],
  }
  // The applications scope is not granted, so the manager would refuse its tool; a server's tool
  // is kept whether or not the server is listed now.
  assert.deepEqual(excludedTools(draft, scopes), [
    "screenshot",
    "playwright__browser_run_code_unsafe",
  ])
  // Without the manager's scopes nothing can be told apart, so every name goes.
  assert.deepEqual(excludedTools(draft), [
    "application_diagnostics",
    "screenshot",
    "playwright__browser_run_code_unsafe",
  ])
  assert.deepEqual(grantRequests([draft], scopes)[0].excluded_tools, [
    "screenshot",
    "playwright__browser_run_code_unsafe",
  ])
  // Nothing left out sends no list, as requests did before there was one.
  assert.ok(!("excluded_tools" in grantRequests([{ ...draft, excluded: [] }], scopes)[0]))
  assert.ok(isServerTool("playwright__browser_run_code_unsafe"))
  assert.ok(!isServerTool("application_diagnostics"))
})

test("scopes and servers read as a few words", () => {
  assert.equal(scopeSummary([]), "No container tools")
  assert.equal(scopeSummary([...SCOPES].reverse()), "All container tools")
  assert.equal(scopeSummary(["files_read", "view"]), "View and Read files")
  assert.equal(serversSummary("all"), "all, including later ones")
  assert.equal(serversSummary([]), "none")
  assert.equal(serversSummary(["chrome", "x"]), "chrome and x")
  // A scope a newer manager added is kept, after the known ones, as it is called.
  assert.deepEqual(orderScopes(["power", "telepathy", "view", "view"]), [
    "view",
    "power",
    "telepathy",
  ])
})

test("an endpoint's URL joins the dashboard's origin and its path", () => {
  const url = endpointUrl("https://thoric.lan/", "/mcp/unity")
  assert.equal(url, "https://thoric.lan/mcp/unity")
  assert.equal(endpointUrl("http://localhost:8080", "mcp/unity"), "http://localhost:8080/mcp/unity")
})

test("the API paths encode the names they carry", () => {
  assert.equal(endpointApi(), "mcp/endpoints")
  assert.equal(endpointApi("unity"), "mcp/endpoints/unity")
  assert.equal(serverApi("a b"), "containers/a%20b/mcp/servers")
  assert.equal(serverApi("c1", "x/y"), "containers/c1/mcp/servers/x%2Fy")
})

test("the server form's text fields become the request's lists and maps", () => {
  assert.deepEqual(parseLines("  -y \r\n\n@playwright/mcp@latest\n"), [
    "-y",
    "@playwright/mcp@latest",
  ])
  assert.deepEqual(parseEnv("API_KEY=${API_KEY}\n\nQUERY=a=b\nEMPTY=\r\n"), {
    API_KEY: "${API_KEY}",
    QUERY: "a=b",
    EMPTY: "",
  })
  assert.throws(() => parseEnv("GOOD=1\nnot a pair"), /Line 2 of the environment/)
  assert.throws(() => parseEnv("1BAD=x"), /Line 1 of the environment/)
  assert.throws(() => parseEnv("A=1\nA=2"), /A is set twice/)
  assert.deepEqual(parseHeaders("Authorization: Bearer ${TOKEN}\nX-Empty:\n"), {
    Authorization: "Bearer ${TOKEN}",
    "X-Empty": "",
  })
  assert.throws(() => parseHeaders("no colon"), /Line 1 of the headers/)
  assert.throws(() => parseHeaders("Bad Name: x"), /Line 1 of the headers/)
  // Header names do not differ by case.
  assert.throws(() => parseHeaders("Accept: a\naccept: b"), /accept is set twice/)
})

test("a server inside the desktop is reached on a loopback address with a port", () => {
  for (const url of [
    "http://127.0.0.1:8931/mcp",
    "https://localhost:443",
    "http://[::1]:9000/sse?x=1",
    "http://127.0.0.1:${PORT}/mcp",
  ])
    assert.ok(localUrl(url), url)
  for (const url of [
    "http://127.0.0.1/mcp",
    "http://example.com:80/mcp",
    "ftp://localhost:21",
    "http://localhost.evil:80",
    "http://127.0.0.1:80 /x",
  ])
    assert.ok(!localUrl(url), url)
})

test("the Add server form sends one way to reach the server", () => {
  const stdio = {
    ...serverDraft(),
    name: " playwright ",
    description: " Browser ",
    command: " npx ",
    args: "-y\n@playwright/mcp@latest",
    env: "DISPLAY=:1",
    cwd: " /home/desktop ",
    // Left from the other transport; it is not sent.
    url: "http://example.com",
  }
  assert.deepEqual(serverRequest(stdio), {
    name: "playwright",
    description: "Browser",
    command: "npx",
    args: ["-y", "@playwright/mcp@latest"],
    env: { DISPLAY: ":1" },
    cwd: "/home/desktop",
  })
  assert.deepEqual(serverRequest({ ...serverDraft(), name: "x", command: "x-mcp" }), {
    name: "x",
    command: "x-mcp",
  })
  assert.deepEqual(
    serverRequest({
      ...stdio,
      transport: "http",
      url: "http://127.0.0.1:8931/mcp",
      headers: "Authorization: Bearer ${TOKEN}",
    }),
    {
      name: "playwright",
      description: "Browser",
      url: "http://127.0.0.1:8931/mcp",
      headers: { Authorization: "Bearer ${TOKEN}" },
    }
  )
  assert.throws(() => serverRequest({ ...stdio, name: "Playwright" }), /lowercase letters/)
  assert.throws(() => serverRequest({ ...stdio, command: " " }), /command that starts/)
  assert.throws(() => serverRequest({ ...stdio, transport: "http" }), /127\.0\.0\.1/)
})

test("a configuration's server declarations are checked like the build checks them", () => {
  const servers = [
    { name: "chrome", command: "chrome-mcp", args: ["--port", "${PORT}"], env: { A: "1" } },
    { name: "x", description: "", url: "http://127.0.0.1:9000/mcp", headers: {} },
  ]
  assert.deepEqual(readServerDeclarations(servers), [
    servers[0],
    // Empty fields are left out.
    { name: "x", url: "http://127.0.0.1:9000/mcp" },
  ])
  const cases: [unknown, RegExp][] = [
    [{ name: "x" }, /list of servers/],
    [[null], /JSON object/],
    [[{ name: "X", command: "x" }], /needs a name/],
    [[{ name: "x", command: 7 }], /x is not shaped/],
    [[{ name: "x", command: "x", args: "--a" }], /x is not shaped/],
    [[{ name: "x", command: "x", env: { A: 1 } }], /x is not shaped/],
    [[{ name: "x" }], /either a command or a URL/],
    [[{ name: "x", command: "x", url: "http://127.0.0.1:1" }], /either a command or a URL/],
    [[{ name: "x", command: "x", headers: { A: "b" } }], /either a command or a URL/],
    [[{ name: "x", url: "http://127.0.0.1:1", cwd: "/tmp" }], /either a command or a URL/],
    [
      [
        { name: "x", command: "a" },
        { name: "x", command: "b" },
      ],
      /x is declared twice/,
    ],
  ]
  for (const [value, message] of cases)
    assert.throws(() => readServerDeclarations(value), message, JSON.stringify(value))
})

test("a server's command reads as one line, its arguments quoted where needed", () => {
  assert.equal(
    serverCommand({ command: "npx", args: ["-y", "@playwright/mcp"] }),
    "npx -y @playwright/mcp"
  )
  assert.equal(serverCommand({ command: "run", args: ["a b", ""] }), 'run "a b" ""')
  assert.equal(serverCommand({ url: "http://127.0.0.1:1/mcp" }), "http://127.0.0.1:1/mcp")
})

const server = (container_id: string, name: string): DesktopServer => ({
  container_id,
  container_name: `name of ${container_id}`,
  name,
  source: "image",
  description: "",
  transport: "stdio",
  command: "x",
  args: [],
  cwd: null,
  env: [],
  url: null,
  headers: [],
  state: "ready",
  error: null,
  tools: [],
  tools_at: null,
  log: "",
})

test("desktop servers are grouped by container, in the containers' order", () => {
  const groups = serversByContainer(
    [server("b", "chrome"), server("gone", "x"), server("b", "x"), server("a", "y")],
    [{ id: "a" }, { id: "b" }, { id: "c" }]
  )
  assert.deepEqual(
    groups.map((group) => [group.id, group.name, group.servers.map((entry) => entry.name)]),
    [
      ["a", "name of a", ["y"]],
      ["b", "name of b", ["chrome", "x"]],
      ["c", "", []],
      // A container the list no longer has keeps its servers, under the name they reported.
      ["gone", "name of gone", ["x"]],
    ]
  )
  assert.equal(groups[3].container, undefined)
})

test("an endpoint's tools are grouped by where they come from", () => {
  const tool = (
    source: EndpointTool["source"],
    container_id: string | null,
    server: string | null
  ) => ({
    name: `${source}-${container_id}-${server}`,
    description: "",
    source,
    container_id,
    server,
  })
  const groups = toolGroups([
    tool("thoric", "a", null),
    tool("server", "a", "chrome"),
    tool("thoric", "a", null),
    tool("server", "b", "chrome"),
    // get_job is the endpoint's own tool and belongs to no container.
    tool("thoric", null, null),
  ])
  assert.deepEqual(
    groups.map((group) => [group.source, group.container_id, group.server, group.tools.length]),
    [
      ["thoric", "a", null, 2],
      ["server", "a", "chrome", 1],
      ["server", "b", "chrome", 1],
      ["thoric", null, null, 1],
    ]
  )
})

test("the create form names the servers the chosen image declares", () => {
  const images = [
    { image_id: "sha256:1", reference: "catalog:1", mcp_servers: [] },
    { image_id: "sha256:2", reference: "saved:2", mcp_servers: ["chrome"] },
  ]
  assert.deepEqual(imageServers(images, "sha256:2"), ["chrome"])
  assert.deepEqual(imageServers(images, "saved:2"), ["chrome"])
  assert.deepEqual(imageServers(images, "sha256:1"), [])
  assert.deepEqual(imageServers(images, undefined), [])
})
