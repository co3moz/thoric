import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"

import { MCP_CLIENTS, mcpClient } from "@/lib/mcp-connect"
import { skillCommands, skillServer, skillsFor, windowsPath, type Skill } from "@/lib/skills"
import type { McpEndpoint, McpGrant } from "@/lib/types"

const skill = (name: string, servers: Skill["servers"]): Skill => ({
  name,
  description: `What ${name} teaches.`,
  url: `/.well-known/agent-skills/${name}/SKILL.md`,
  digest: `sha256:${"0".repeat(64)}`,
  size: 1,
  servers,
})
const scaleSets = skill("thoric-scale-sets", ["root", "set"])

const grant = (set: string | null): McpGrant => ({
  container_id: set ? "" : "c1",
  set,
  container_name: set ?? "chrome",
  alias: set ?? "chrome",
  scopes: ["view"],
  servers: "all",
  excluded_tools: [],
})
const endpoint = (grants: McpGrant[]): McpEndpoint => ({
  name: "re",
  path: "/mcp/re",
  automatic: false,
  has_token: true,
  token_created_at: 1,
  created_at: 1,
  grants,
  tool_count: 1,
})

test("every client names the folders it reads skills from, or says it has none", () => {
  for (const client of MCP_CLIENTS) {
    if (client.skills === null) continue
    const { user, project } = client.skills
    assert.match(user, /^~\/[\w./-]+\/skills$/, client.id)
    assert.match(project, /^\.[\w-]+\/(?:[\w-]+\/)*skills$/, client.id)
  }
  // The folder every client of the Agent Skills standard shares stands for the ones not listed.
  assert.deepEqual(mcpClient("generic").skills, {
    user: "~/.agents/skills",
    project: ".agents/skills",
  })
  assert.equal(mcpClient("claude-desktop").skills, null)
})

test("/mcp and a scale set's endpoint get their skills, a container's endpoint none", () => {
  const containerOnly = skill("thoric-desktops", ["root"])
  const skills = [scaleSets, containerOnly]
  assert.equal(skillServer(undefined), "root")
  assert.equal(skillServer("re", endpoint([grant("re")])), "set")
  assert.equal(skillServer("chrome", endpoint([grant(null)])), null)
  // Until the endpoint list has loaded, nothing is offered.
  assert.equal(skillServer("re", undefined), null)
  assert.deepEqual(skillsFor(skills, "root"), skills)
  assert.deepEqual(skillsFor(skills, "set"), [scaleSets])
  assert.deepEqual(skillsFor(skills, null), [])
})

test("the commands copy each skill from the dashboard's address into the client's folder", () => {
  const folder = mcpClient("claude-code").skills!
  const [shell, powershell] = skillCommands(folder, "http://localhost:8080", [scaleSets])
  assert.equal(shell.kind, "shell")
  assert.equal(
    shell.text,
    "mkdir -p ~/.claude/skills/thoric-scale-sets && curl -fsSL http://localhost:8080/.well-known/agent-skills/thoric-scale-sets/SKILL.md -o ~/.claude/skills/thoric-scale-sets/SKILL.md"
  )
  assert.equal(powershell.kind, "powershell")
  assert.equal(
    powershell.text,
    'New-Item -ItemType Directory -Force "$HOME\\.claude\\skills\\thoric-scale-sets" | Out-Null; Invoke-WebRequest -UseBasicParsing "http://localhost:8080/.well-known/agent-skills/thoric-scale-sets/SKILL.md" -OutFile "$HOME\\.claude\\skills\\thoric-scale-sets\\SKILL.md"'
  )
  // A dashboard behind the HTTPS proxy on another name is the address the commands use, and
  // every skill gets a line of its own.
  const two = skillCommands(folder, "https://thoric.lan", [scaleSets, skill("other", ["root"])])
  for (const snippet of two) {
    const lines = snippet.text.split("\n")
    assert.equal(lines.length, 2, snippet.kind)
    assert.ok(lines.every((line) => line.includes("https://thoric.lan/.well-known/agent-skills/")))
  }
})

test("the MCP guide names the folders the dialog gives each client", () => {
  const guide = readFileSync(new URL("../../../docs/MCP.md", import.meta.url), "utf8")
  for (const client of MCP_CLIENTS) {
    if (client.skills === null) continue
    for (const folder of [client.skills.user, client.skills.project]) {
      assert.ok(guide.includes(`\`${folder}/\``), `docs/MCP.md must name ${folder} of ${client.id}`)
    }
  }
})

test("a home path reads as PowerShell writes it", () => {
  assert.equal(windowsPath("~/.config/opencode/skills"), "$HOME\\.config\\opencode\\skills")
  assert.equal(windowsPath(".agents/skills"), ".agents\\skills")
})
