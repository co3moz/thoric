// How MCP clients add an endpoint of this manager, or Thoric MCP at `/mcp`: a link, a command
// or a settings file for each client, made in the browser from the URL and the token typed into
// the dialog. Nothing here reaches the manager. What a client writes is in its own format and
// stays English, like everything else sent to another program.
import type { CodeLanguage } from "./highlight"

/** What a client connects to. */
export type ConnectTarget = {
  /** The name the client lists the server by: the endpoint's, or `thoric` for `/mcp`. */
  name: string
  /** Where the dashboard reaches it, such as `http://localhost:8080/mcp/chrome-dev`. */
  url: string
  /** Its path on the manager, `/mcp` or `/mcp/NAME`, which the stdio bridge reaches from inside. */
  path: string
  /** The bearer token, or `null` when the client sends no Authorization header. */
  token: string | null
  /** The manager's container, which the stdio bridge runs in: `MANAGER_CONTAINER`, when known. */
  container?: string
}

/** The manager's container unless `MANAGER_CONTAINER` names another, as in the Compose manifests. */
export const MANAGER_CONTAINER = "thoric-manager"

/** Stand-ins for a token the dialog was not given; the documentation uses the same words. */
export const ENDPOINT_TOKEN = "ENDPOINT_TOKEN"
export const ADMIN_TOKEN = "YOUR_MCP_TOKEN"

/** What a snippet is, which names it in the dialog. */
export type SnippetKind =
  /** A command to run in a terminal. */
  | "command"
  /** An entry for the client's settings file. */
  | "file"
  /** A settings entry that runs the stdio bridge in the manager through WSL. */
  | "wsl"
  /** A settings entry that runs mcp-remote, for clients that start only local programs. */
  | "remote"
  /** A command for a POSIX shell: macOS, Linux and WSL. */
  | "shell"
  /** A command for Windows PowerShell. */
  | "powershell"

export type Snippet = {
  kind: SnippetKind
  language: CodeLanguage
  /** For an entry, the files it goes into, such as the user's and a project's. */
  paths?: string[]
  text: string
}

export type ClientId =
  | "claude-code"
  | "codex"
  | "vscode"
  | "copilot-cli"
  | "cursor"
  | "opencode"
  | "gemini"
  | "devin"
  | "zed"
  | "cline"
  | "kiro"
  | "amp"
  | "claude-desktop"
  | "generic"

export type McpClient = {
  id: ClientId
  /** The product's name, the same in every language. */
  name: string
  /** Its documentation about MCP servers. */
  docs: string
  /** Flags and file names its hint shows as code, by the hint's placeholder. */
  terms?: Record<string, string>
  /** A link that opens the client and adds the server, when the client has one. */
  install?: (target: ConnectTarget) => string
  snippets: (target: ConnectTarget) => Snippet[]
  /** Where it loads Agent Skills from by itself, or `null` for a client without such a folder. */
  skills: SkillsFolder | null
}

/**
 * The folders a client reads skills from, a folder per skill named after it, as its own
 * documentation names them.
 */
export type SkillsFolder = {
  /** Every project's, from the user's home. */
  user: string
  /** One project's, from its root. */
  project: string
}

const json = (value: unknown) => JSON.stringify(value, null, 2)

/** The Authorization header as a JSON settings entry names it, or nothing without a token. */
const headers = (target: ConnectTarget) =>
  target.token === null ? {} : { headers: { Authorization: `Bearer ${target.token}` } }

/** `--header "Authorization: Bearer TOKEN"` for a command, or nothing without a token. */
const headerFlag = (target: ConnectTarget) =>
  target.token === null ? [] : ["--header", shellWord(`Authorization: Bearer ${target.token}`)]

/**
 * An argument for a terminal, quoted only when it needs to be. Double quotes read the same in a
 * POSIX shell, PowerShell and cmd; a value they would change gets POSIX single quotes instead.
 */
export function shellWord(value: string) {
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(value)) return value
  if (!/["$`\\!]/.test(value)) return `"${value}"`
  return `'${value.replace(/'/g, `'\\''`)}'`
}

/** A string as TOML writes one: JSON's escapes are TOML's for every character a URL holds. */
const tomlString = (value: string) => JSON.stringify(value)

/** A TOML key, bare when TOML allows it, as an endpoint's name always is. */
const tomlKey = (value: string) => (/^[A-Za-z0-9_-]+$/.test(value) ? value : tomlString(value))

/** Base64 of the text's UTF-8 bytes, which `btoa` alone refuses beyond Latin-1. */
const base64 = (text: string) =>
  btoa(Array.from(new TextEncoder().encode(text), (byte) => String.fromCharCode(byte)).join(""))

/**
 * A settings entry that runs `core/bridge/mcp_bridge.py` in the manager through WSL. The bridge
 * reaches the manager on its own port from inside the container and uses the manager's own MCP
 * token, which opens every endpoint too, so no token is written into the settings.
 */
export function wslBridge(target: ConnectTarget) {
  const inside =
    target.path === "/mcp" ? [] : ["-e", `MCP_BRIDGE_URL=http://127.0.0.1:8080${target.path}`]
  return {
    command: "wsl.exe",
    args: [
      "-d",
      "Thoric",
      "--",
      "docker",
      "exec",
      "-i",
      ...inside,
      target.container || MANAGER_CONTAINER,
      "python3",
      "/app/mcp_bridge.py",
    ],
  }
}

/**
 * A settings entry that runs mcp-remote with Node.js. The header goes through an environment
 * variable without a space after the colon, since some clients split arguments at spaces on
 * Windows. mcp-remote takes plain HTTP only from `localhost` and `127.0.0.1` unless told otherwise.
 */
export function mcpRemote(target: ConnectTarget) {
  const local = /^http:\/\/(localhost|127\.0\.0\.1)(:|\/|$)/i.test(target.url)
  const args = ["-y", "mcp-remote", target.url]
  if (target.url.startsWith("http:") && !local) args.push("--allow-http")
  if (target.token === null) return { command: "npx", args }
  return {
    command: "npx",
    args: [...args, "--header", "Authorization:${AUTH_HEADER}"],
    env: { AUTH_HEADER: `Bearer ${target.token}` },
  }
}

/** The clients the dialog offers, the most used first; `generic` stays last. */
export const MCP_CLIENTS: McpClient[] = [
  {
    id: "claude-code",
    name: "Claude Code",
    docs: "https://code.claude.com/docs/en/mcp",
    skills: { user: "~/.claude/skills", project: ".claude/skills" },
    terms: { flag: "--scope user", file: ".mcp.json" },
    snippets: (target) => [
      {
        kind: "command",
        language: "bash",
        text: [
          "claude mcp add --transport http",
          shellWord(target.name),
          shellWord(target.url),
          ...headerFlag(target),
        ].join(" "),
      },
      {
        kind: "file",
        language: "json",
        paths: [".mcp.json"],
        text: json({
          mcpServers: { [target.name]: { type: "http", url: target.url, ...headers(target) } },
        }),
      },
    ],
  },
  {
    id: "codex",
    name: "Codex",
    docs: "https://learn.chatgpt.com/docs/extend/mcp?surface=cli",
    skills: { user: "~/.agents/skills", project: ".agents/skills" },
    terms: { file: "config.toml" },
    // `codex mcp add` takes a token only from an environment variable, and `bearer_token` in the
    // file is refused, so the token goes in as a header; the command is for a server without one.
    snippets: (target) => [
      ...(target.token === null
        ? [
            {
              kind: "command" as const,
              language: "bash" as const,
              text: `codex mcp add ${shellWord(target.name)} --url ${shellWord(target.url)}`,
            },
          ]
        : []),
      {
        kind: "file",
        language: "toml",
        paths: ["~/.codex/config.toml"],
        text: [
          `[mcp_servers.${tomlKey(target.name)}]`,
          `url = ${tomlString(target.url)}`,
          ...(target.token === null
            ? []
            : [`http_headers = { "Authorization" = ${tomlString(`Bearer ${target.token}`)} }`]),
        ].join("\n"),
      },
    ],
  },
  {
    id: "vscode",
    name: "VS Code",
    docs: "https://code.visualstudio.com/docs/copilot/customization/mcp-servers",
    skills: { user: "~/.copilot/skills", project: ".github/skills" },
    terms: { file: ".vscode/mcp.json" },
    install: (target) =>
      `vscode:mcp/install?${encodeURIComponent(
        JSON.stringify({ name: target.name, type: "http", url: target.url, ...headers(target) })
      )}`,
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: [".vscode/mcp.json"],
        text: json({
          servers: { [target.name]: { type: "http", url: target.url, ...headers(target) } },
        }),
      },
    ],
  },
  {
    id: "copilot-cli",
    name: "Copilot CLI",
    docs: "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers",
    skills: { user: "~/.copilot/skills", project: ".github/skills" },
    snippets: (target) => [
      {
        kind: "command",
        language: "bash",
        text: [
          "copilot mcp add --transport http",
          ...headerFlag(target),
          shellWord(target.name),
          shellWord(target.url),
        ].join(" "),
      },
      {
        kind: "file",
        language: "json",
        paths: ["~/.copilot/mcp-config.json"],
        text: json({
          mcpServers: {
            [target.name]: { type: "http", url: target.url, ...headers(target), tools: ["*"] },
          },
        }),
      },
    ],
  },
  {
    id: "cursor",
    name: "Cursor",
    docs: "https://cursor.com/docs/context/mcp",
    skills: { user: "~/.cursor/skills", project: ".cursor/skills" },
    install: (target) =>
      `cursor://anysphere.cursor-deeplink/mcp/install?name=${encodeURIComponent(
        target.name
      )}&config=${encodeURIComponent(base64(JSON.stringify({ url: target.url, ...headers(target) })))}`,
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: ["~/.cursor/mcp.json", ".cursor/mcp.json"],
        text: json({ mcpServers: { [target.name]: { url: target.url, ...headers(target) } } }),
      },
    ],
  },
  {
    id: "opencode",
    name: "OpenCode",
    docs: "https://opencode.ai/docs/mcp-servers/",
    skills: { user: "~/.config/opencode/skills", project: ".opencode/skills" },
    terms: { file: "opencode.json" },
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: ["~/.config/opencode/opencode.json", "opencode.json"],
        text: json({
          $schema: "https://opencode.ai/config.json",
          mcp: {
            [target.name]: {
              type: "remote",
              url: target.url,
              enabled: true,
              // A server that answers 401 would otherwise start a sign-in it does not offer.
              ...(target.token === null ? {} : { oauth: false }),
              ...headers(target),
            },
          },
        }),
      },
    ],
  },
  {
    id: "gemini",
    name: "Gemini CLI",
    docs: "https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md",
    skills: { user: "~/.gemini/skills", project: ".gemini/skills" },
    terms: { flag: "--scope user" },
    snippets: (target) => [
      {
        kind: "command",
        language: "bash",
        text: [
          "gemini mcp add --transport http",
          ...headerFlag(target),
          shellWord(target.name),
          shellWord(target.url),
        ].join(" "),
      },
      {
        kind: "file",
        language: "json",
        paths: ["~/.gemini/settings.json", ".gemini/settings.json"],
        text: json({ mcpServers: { [target.name]: { httpUrl: target.url, ...headers(target) } } }),
      },
    ],
  },
  {
    id: "devin",
    name: "Devin Desktop",
    docs: "https://docs.devin.ai/desktop/cascade/mcp",
    skills: { user: "~/.config/devin/skills", project: ".devin/skills" },
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: [
          "~/.config/devin/mcp_config.json",
          "%APPDATA%\\devin\\mcp_config.json",
          "~/.codeium/windsurf/mcp_config.json",
        ],
        text: json({
          mcpServers: { [target.name]: { serverUrl: target.url, ...headers(target) } },
        }),
      },
    ],
  },
  {
    id: "zed",
    name: "Zed",
    docs: "https://zed.dev/docs/ai/mcp",
    skills: { user: "~/.agents/skills", project: ".agents/skills" },
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: ["~/.config/zed/settings.json", "%APPDATA%\\Zed\\settings.json"],
        text: json({ context_servers: { [target.name]: { url: target.url, ...headers(target) } } }),
      },
    ],
  },
  {
    id: "cline",
    name: "Cline",
    docs: "https://docs.cline.bot/mcp/configuring-mcp-servers",
    skills: { user: "~/.cline/skills", project: ".cline/skills" },
    snippets: (target) => [
      {
        kind: "command",
        language: "bash",
        text: [
          "cline mcp add",
          shellWord(target.name),
          shellWord(target.url),
          "--transport http",
          ...headerFlag(target),
          "--yes",
        ].join(" "),
      },
      {
        kind: "file",
        language: "json",
        paths: ["~/.cline/data/settings/cline_mcp_settings.json"],
        // Without `type`, Cline falls back to the older SSE transport.
        text: json({
          mcpServers: {
            [target.name]: { type: "streamableHttp", url: target.url, ...headers(target) },
          },
        }),
      },
    ],
  },
  {
    id: "kiro",
    name: "Kiro",
    docs: "https://kiro.dev/docs/mcp/configuration/",
    skills: { user: "~/.kiro/skills", project: ".kiro/skills" },
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: ["~/.kiro/settings/mcp.json", ".kiro/settings/mcp.json"],
        text: json({ mcpServers: { [target.name]: { url: target.url, ...headers(target) } } }),
      },
    ],
  },
  {
    id: "amp",
    name: "Amp",
    docs: "https://ampcode.com/docs/customize/mcp",
    skills: { user: "~/.config/agents/skills", project: ".agents/skills" },
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        paths: ["~/.config/amp/settings.json", ".amp/settings.json"],
        text: json({
          "amp.mcpServers": { [target.name]: { url: target.url, ...headers(target) } },
        }),
      },
    ],
  },
  {
    id: "claude-desktop",
    name: "Claude Desktop",
    docs: "https://modelcontextprotocol.io/docs/develop/connect-local-servers",
    skills: null,
    snippets: (target) => [
      {
        kind: "wsl",
        language: "json",
        paths: ["%APPDATA%\\Claude\\claude_desktop_config.json"],
        text: json({ mcpServers: { [target.name]: wslBridge(target) } }),
      },
      {
        kind: "remote",
        language: "json",
        paths: [
          "~/Library/Application Support/Claude/claude_desktop_config.json",
          "%APPDATA%\\Claude\\claude_desktop_config.json",
        ],
        text: json({ mcpServers: { [target.name]: mcpRemote(target) } }),
      },
    ],
  },
  {
    id: "generic",
    name: "Other",
    docs: "https://modelcontextprotocol.io/clients",
    skills: { user: "~/.agents/skills", project: ".agents/skills" },
    snippets: (target) => [
      {
        kind: "file",
        language: "json",
        text: json({
          mcpServers: { [target.name]: { type: "http", url: target.url, ...headers(target) } },
        }),
      },
      {
        kind: "wsl",
        language: "json",
        text: json({ mcpServers: { [target.name]: wslBridge(target) } }),
      },
    ],
  },
]

/** A client by its id, or the first one for an id this release does not know. */
export const mcpClient = (id: string | null | undefined) =>
  MCP_CLIENTS.find((client) => client.id === id) ?? MCP_CLIENTS[0]
