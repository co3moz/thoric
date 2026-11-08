// The skills Thoric serves its agents (`GET /api/skills`), which MCP server offers each, and the
// commands that copy one into a client's skills folder from the manager's own address, where the
// web serves every skill without a session. What a command writes stays English, like everything
// else sent to another program.
import { shellWord, type SkillsFolder, type Snippet } from "./mcp-connect"
import type { McpEndpoint } from "./types"

/** The MCP servers of the manager that offer skills: `/mcp`, and a scale set's endpoints. */
export type SkillServer = "root" | "set"

export type Skill = {
  /** Its name, which is also its folder's in a skills folder. */
  name: string
  /** When an agent uses it. */
  description: string
  /** Where the web serves its `SKILL.md`, from the root of the manager's address. */
  url: string
  /** `sha256:` and the hex of the file's SHA-256. */
  digest: string
  /** The file's length in bytes. */
  size: number
  servers: SkillServer[]
}

/**
 * The server an agent connects to: `/mcp` without an endpoint, a set's endpoint when its grant
 * covers a scale set, and `null` for a container's endpoint, which offers no skill.
 */
export function skillServer(name: string | undefined, endpoint?: McpEndpoint): SkillServer | null {
  if (!name) return "root"
  return endpoint?.grants.some((grant) => grant.set) ? "set" : null
}

/** The skills an agent of `server` should have. */
export const skillsFor = (skills: Skill[], server: SkillServer | null) =>
  server ? skills.filter((skill) => skill.servers.includes(server)) : []

/** A path from the home folder as PowerShell writes it, `~/.claude/skills` as `$HOME\.claude\skills`. */
export const windowsPath = (path: string) => path.replace(/^~/, "$HOME").replaceAll("/", "\\")

/**
 * The commands that copy each skill into the user's skills folder of a client, one line per
 * skill: for a POSIX shell, and for Windows PowerShell, whose `Invoke-WebRequest` writes the
 * bytes as they arrive.
 */
export function skillCommands(folder: SkillsFolder, origin: string, skills: Skill[]): Snippet[] {
  const lines = (line: (skill: Skill, url: string) => string) =>
    skills.map((skill) => line(skill, new URL(skill.url, origin).href)).join("\n")
  return [
    {
      kind: "shell",
      language: "bash",
      text: lines((skill, url) => {
        const target = `${folder.user}/${skill.name}`
        return `mkdir -p ${target} && curl -fsSL ${shellWord(url)} -o ${target}/SKILL.md`
      }),
    },
    {
      kind: "powershell",
      language: "bash",
      text: lines((skill, url) => {
        const target = windowsPath(`${folder.user}/${skill.name}`)
        return `New-Item -ItemType Directory -Force "${target}" | Out-Null; Invoke-WebRequest -UseBasicParsing "${url}" -OutFile "${target}\\SKILL.md"`
      }),
    },
  ]
}
