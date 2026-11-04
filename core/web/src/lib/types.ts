// The JSON the manager's HTTP API returns; see docs/API.md.

export type Mount = { volume: string; target: string; readonly: boolean }

/**
 * The proxy a container's programs go through: the installation's, or one of its own whose
 * `password` says only whether one is kept. `null` is none.
 */
export type ProxyChoice =
  | { mode: "installation" }
  | { mode: "custom"; url: string; username?: string | null; bypass: string[]; password?: boolean }

export type ContainerSpec = {
  name: string
  image: string
  resolution: string
  gpu: boolean
  internet: boolean
  audio: boolean
  microphone: boolean
  sudo: boolean
  gamepad: boolean
  /** NET_ADMIN and /dev/net/tun, for a VPN client inside the desktop. */
  vpn: boolean
  /** Absent only before a container exists; `null` is none. */
  proxy?: ProxyChoice | null
  /** The machine identity; `null` on containers created before identities were kept. */
  hostname?: string | null
  mac_address?: string | null
  machine_id?: string | null
  /** Ports inside the desktop that the manager proxies for holders of the MCP token. */
  ports: number[]
  /** Groups whose members reach each other on the group's network; absent before groups existed. */
  groups?: string[]
  /** More names the members of its groups reach the container by. */
  aliases?: string[]
  cpus: number | null
  memory_mb: number | null
  shm_mb: number
  mounts: Mount[]
  /** When Thoric stops or deletes the container by itself; absent before a container exists. */
  lifecycle?: Lifecycle
}

/** What an expired container becomes. */
export type OnExpiry = "stop" | "delete"

/** A container's lifecycle policy, as the manager keeps it. */
export type Lifecycle = {
  /** Minutes the desktop may go unused before it is stopped; 0 never. */
  idle_stop_minutes: number
  /** When the container expires, in Unix seconds; `null` never. */
  expires_at: number | null
  on_expiry: OnExpiry
  /** With `on_expiry` delete, the volumes created for it go too. */
  delete_volumes: boolean
}

/** What an image's labels say about its desktop; `catalog_id` names the catalog image it derives from. */
export type DesktopInfo = { catalog_id: string; title: string; session: string }

export type Container = {
  id: string
  /** The Docker container's name. */
  container: string
  source_image_id: string
  created_at: number
  spec: ContainerSpec
  last_error?: string | null
  desktop?: DesktopInfo | null
  /** Set when the image had a compose: its setup's progress and the values it was given. */
  setup?: ContainerSetup | null
}

/** A compose parameter: an environment variable of the setup script. */
export type ParameterType =
  "text" | "number" | "boolean" | "choice" | "choices" | "volume" | "folder"

export type ParameterDisplay =
  "input" | "multiline" | "slider" | "toggle" | "checkbox" | "select" | "radio" | "checkboxes"

export type ParameterOption = { value: string; label: string }

/**
 * A compose parameter: an environment variable of the setup script. Whatever its type, its value
 * and default are text in the form the script reads: `42`, `true`, or a choice per line. A volume
 * parameter's value names a volume, or is empty for none; the script reads its `path`. A folder
 * parameter's value is an absolute folder of the desktop, which uploads can go into.
 */
export type ComposeParameter = {
  name: string
  label: string
  /** Help shown with the field, such as where to find the value. */
  description: string
  type: ParameterType
  /** The type's first display when left out. */
  display?: ParameterDisplay | null
  /** A boolean that is required must be turned on. */
  required: boolean
  secret: boolean
  default: string
  options?: ParameterOption[]
  allow_custom?: boolean
  min?: number | null
  max?: number | null
  step?: number | null
  /** Volume parameters: where the volume is mounted in the desktop. */
  path?: string
  /** Text parameters: a regular expression, in Rust's syntax, a value must match as a whole. */
  pattern?: string
  /** Text parameters with a pattern: the expected format, shown when a value does not match. */
  pattern_message?: string
  /** Volume parameters without a default: an empty value gets the container a volume of its own. */
  new_volume?: boolean
}

/** One script of a compose, with the parameters it asks for. */
export type ComposeStep = {
  /** Names the step in a compose of several, such as the template it came from. */
  id?: string
  /** What the forms call the step; its id when empty. */
  name?: string
  parameters: ComposeParameter[]
  script: string
  root: boolean
  timeout_seconds: number
}

/**
 * What a new container from an image asks for, and the scripts that receive it: one step with
 * its fields at the top, as composes were before steps, or several in `steps`. Each script
 * receives only its own step's parameters; with several steps a value goes by `<step>.<NAME>`.
 */
export type Compose = ComposeStep | { steps: ComposeStep[] }

/** A compose as image lists show it, without its scripts. */
export type ListedCompose = Omit<ComposeStep, "script"> | { steps: Omit<ComposeStep, "script">[] }

export type SetupStatus = "pending" | "running" | "completed" | "failed"

export type ContainerSetup = {
  status: SetupStatus
  /** The image record the compose came from. */
  image: string
  /** Values of the parameters that are not secret, by the names they go by. */
  parameters: Record<string, string>
  /** Names of the secret parameters, whose values are never returned. */
  secrets: string[]
  finished_at?: number | null
  /** The volumes its volume parameters were given, mounted beside the settings' own. */
  mounts?: Mount[]
  /** The folders its volume and folder parameters name, which take uploads. */
  places?: Place[]
  /** The steps of a compose of several, in order, and where each stands. */
  steps?: { id: string; name: string; status: SetupStatus }[]
}

/** A folder of the desktop that a compose names, by its parameter's label. */
export type Place = { label: string; path: string }

export type Health = "healthy" | "unhealthy" | "starting" | "unknown"

/** A container record with Docker's view of it. */
export type ContainerItem = Container & {
  /** Docker's container state, `missing` without a container or `unavailable` without Docker. */
  state: string
  health: Health
  status: string | null
  /** Whether its VNC server stays in its private network, away from its groups; `null` without a container. */
  vnc_private?: boolean | null
  /** Whether something uses its running desktop now, which its idle stop waits for. */
  in_use?: boolean
  /**
   * When its idle stop comes if nothing uses it from now on, in Unix seconds, a whole idle time
   * ahead while it is in use; `null` without one or while it is stopped.
   */
  idle_stop_at?: number | null
}

/**
 * An image as `GET images` lists it: the catalog images published with Thoric first, then the
 * saved ones. `kind` is `catalog`, `build`, `snapshot`, `import` or `archive`.
 */
export type ImageItem = {
  /** A catalog image's catalog ID, such as `ubuntu-xfce`, or a saved image's record ID. */
  id: string
  name: string
  /** Docker's image ID; `null` for a catalog image that is not downloaded. */
  image_id: string | null
  reference: string
  parent_image_id: string | null
  source_container: string | null
  kind: string
  /** `null` for a catalog image. */
  created_at: number | null
  description: string
  desktop?: DesktopInfo | null
  /** Recorded by script builds; `GET images/{id}/config` adds the scripts. */
  build?:
    | (Omit<BuildConfig, "name" | "script" | "parts"> & {
        parts?: Omit<BuildPart, "script">[]
      })
    | null
  /** Lists leave out the setup scripts; `GET images/{id}/compose` returns them. */
  compose?: ListedCompose | null
  /**
   * Whether a container can start from it now, `null` when Docker could not list images. A
   * catalog image that is not downloaded is not available, but downloads when it is used.
   */
  available?: boolean | null
  /** Catalog images: whether Docker has the reference, even one whose labels do not match. */
  downloaded?: boolean
  /** Catalog images: why a downloaded reference cannot be used. */
  error?: string | null
  /** Saved images with an icon: its version; `GET images/{id}/icon` returns the PNG. */
  icon?: string | null
  /** The templates page templates the image was built from, in the order they ran. */
  template_ids?: string[]
  /** Bytes as Docker reports them; `null` when Docker does not have the image. */
  size?: number | null
  /** What the image's layers hold unpacked. */
  layers_size?: number | null
  /** The part of `layers_size` no other listed image brought. */
  own_size?: number | null
  /** The desktop MCP servers the image declares, which every container from it has. */
  mcp_servers?: string[]
  /**
   * A build that lacks steps of the compose of the saved image it was built on, as builds before
   * they kept it did; `POST images/{id}/compose/base` adds them.
   */
  base_compose_missing?: boolean
}

/** One step of an image's history, as `GET images/{id}/layers` lists it. */
export type ImageStep = {
  /** The step as a Dockerfile says it; empty for a commit that recorded no command. */
  instruction: string
  kind: "run" | "copy" | "add" | "config" | "commit"
  /** Bytes the step wrote, unpacked. */
  size: number
  /** Unix seconds. */
  created: number | null
  /** A commit's message, such as the snapshot it saved. */
  comment?: string
  /** Whether the step wrote one of the image's layers. */
  layer: boolean
  /** The list ID of the image the step came from; `null` for the image's own. */
  from: string | null
  /** The list IDs of the other images that have the step. */
  shared_with: string[]
}

/** What an image's disk space is made of; its steps are the newest first. */
export type ImageLayers = {
  id: string
  image_id: string
  size: number
  layers_size: number
  own_size: number
  steps: ImageStep[]
}

/** One template's script in a build merged from several, run in a layer of its own. */
export type BuildPart = { id: string; name: string; script: string }

/**
 * A choice a template offers when it is installed, such as an optional tool: on or off, written
 * as `NAME=true` or `NAME=false` at the top of its script when it builds. The manager never sees
 * options; a request carries the script with the values in it.
 */
export type TemplateOption = {
  /** A shell variable name the script reads, such as `RADARE2`. */
  name: string
  label: string
  description?: string
  default: boolean
  /** Desktop MCP servers of the template that are left out of the image while it is off. */
  mcp?: string[]
}

/** A part as the build form holds it, with its template's options, which are never sent. */
export type DraftPart = BuildPart & { options?: TemplateOption[] }

/**
 * A build request, which is also what an image's exported configuration holds and what a
 * template on the templates page is. It runs a script, or the parts of templates merged into
 * one image.
 */
export type BuildConfig = {
  name: string
  description?: string
  base_image: string
  internet: boolean
  script?: string
  parts?: BuildPart[]
  compose?: Compose
  /** `false` leaves out the compose of a saved base image, which a build keeps before its own. */
  inherit_compose?: boolean
  /** A PNG of at most 256 by 256 pixels as a `data:image/png;base64,` URL. */
  icon?: string
  /** One template the image is built from. */
  template_id?: string
  /** The templates merged into the image, in order. */
  template_ids?: string[]
  /** Desktop MCP servers written into the image, which every container from it has. */
  mcp?: ServerDeclaration[]
}

/** One line of `docker volume ls --format json`, and whether it is the manager's own. */
export type Volume = {
  Name: string
  Driver: string
  Scope: string
  Labels?: string
  Mountpoint?: string
  /** One of the manager's own volumes, such as its data volume: never mounted or opened. */
  manager?: boolean
  /** The container a compose created this volume for; `null` for any other volume. */
  owner?: VolumeOwner | null
}

export type VolumeOwner = {
  container_id: string
  /** The compose parameter the volume was created for. */
  parameter: string | null
  /** The container's name, or `null` once it was deleted. */
  name: string | null
  deleted: boolean
}

export type JobStatus = "queued" | "running" | "completed" | "failed" | "interrupted"

export type DownloadProgress = {
  current?: number
  total?: number | null
  layers?: number
  layers_done?: number
  /** Set for an archive transfer or a volume copy, whose progress counts bytes instead of layers. */
  transfer?: "export" | "import" | "copy"
  /** Set while a build's or a compose setup's scripts run. */
  script?: ScriptSummary
}

/**
 * Where the scripts of a build or a compose setup are: the part that started last, the step it
 * named last (`THORIC_STAGE`, `THORIC_SKIP`) or else the line it reached, and the share done.
 */
export type ScriptSummary = {
  part: number
  parts: number
  /** Only when there are several parts. */
  name?: string
  stage?: number
  stages?: number
  label?: string
  skipped?: boolean
  line?: number
  lines?: number
  percent: number
}

/** A step a script named; a skipped one has no times. */
export type ScriptStep = {
  stage: number
  label?: string
  started_at?: number | null
  finished_at?: number | null
  skipped?: boolean
}

/** A build part or a compose step. One a failed job stopped in has no `finished_at`. */
export type ScriptPart = {
  name?: string
  started_at?: number | null
  finished_at?: number | null
  cached?: boolean
  stages?: number
  steps?: ScriptStep[]
}

export type ScriptReport = { parts: ScriptPart[] }

/** A named part of a job, such as a script build's layer save; times are the manager's seconds. */
export type JobStage = {
  name: string
  started_at?: number | null
  finished_at?: number | null
}

export type Job = {
  id: string
  kind: string
  resource?: string | null
  /** What a job that makes a container or an image calls it, before its record exists. */
  name?: string | null
  status: JobStatus
  created_at: number
  finished_at: number | null
  log: string
  result?: Record<string, unknown> | null
  progress?: DownloadProgress | null
  /** Every stage of a job that reports them, in order; one that has not started has no times. */
  stages?: JobStage[]
  /** The parts and named steps of a build's or a compose setup's scripts; lists omit it. */
  script?: ScriptReport | null
  /** A failed or interrupted job the manager can start again with its request; lists omit it. */
  retryable?: boolean
  /** A queued job, or a running script, build or image download, that a cancel stops; lists omit it. */
  cancellable?: boolean
}

/** How a GPU desktop reaches the host's GPU: WSL_GPU, NVIDIA_COMPUTE or DRI_GPU. */
export type GpuRoute = "wsl" | "nvidia" | "dri"

/** Why a route its setting allows is not used. */
export type GpuMissing =
  "no_wsl_gpu" | "no_nvidia_gpu" | "no_toolkit" | "no_render_device" | "beside_wsl" | "unseen"

export type GpuReport = {
  /** The routes GPU desktops get. */
  routes: GpuRoute[]
  /** The routes their settings allow that GPU desktops do not get, and why. */
  unused: { route: GpuRoute; setting: string; reason: GpuMissing }[]
  /** What the host showed when the manager started, or `null` where the manager cannot see it. */
  found: { wsl: boolean; nvidia: boolean; toolkit: boolean | null; render: string[] } | null
}

export type Settings = {
  /** The manager's release, such as `0.4.0`. */
  version: string
  base_image: string
  audio_enabled: boolean
  microphone_enabled: boolean
  /** Whether GPU desktops get a GPU: a route the settings allow that the host has. */
  gpu_enabled: boolean
  wsl_gpu: boolean
  nvidia_compute: boolean
  dri_gpu?: boolean
  gpu_adapter?: string | null
  gpu?: GpuReport
  upload_limit_bytes: number
  /** The largest unpacked image archive an import accepts. */
  archive_limit_bytes: number
  /** The template list the templates page reads, or `null` when the page is turned off. */
  templates_url: string | null
  /** The architecture of the images the Docker host runs, such as `amd64` or `arm64`. */
  architecture?: string
  /** The label a volume needs to be listed and mounted, or `null` when every volume is. */
  volume_filter: string | null
  /** Whether the root /mcp answers agents (ROOT_MCP_ENABLED); MCP endpoints answer either way. */
  root_mcp_enabled: boolean
  /** Whether the installation allows these container features (INTERNET_ENABLED and so on). */
  internet_enabled?: boolean
  sudo_enabled?: boolean
  gamepad_enabled?: boolean
  vpn_enabled?: boolean
  /** The installation's proxy (PROXY_URL), without its password; `null` when it has none. */
  proxy?: { url: string; username: string | null; bypass: string[] } | null
  /** What a new container gets of each feature unless it asks otherwise. */
  defaults?: Record<
    "gpu" | "internet" | "sudo" | "audio" | "microphone" | "gamepad" | "vpn",
    boolean
  > & {
    /** The idle stop a new container gets, in minutes; 0 never (IDLE_STOP_DEFAULT). */
    idle_stop_minutes?: number
  }
}

/** `GET update`: this release, the newest one and whether the dashboard can update to it. */
export type UpdateStatus = {
  /** Whether the manager looks for new releases (UPDATE_CHECK). */
  enabled: boolean
  current: string
  /**
   * The commit of an `edge` build of master, which still carries the version of the release
   * before it; `null` for a release or a build from the source.
   */
  commit: string | null
  /** The newest release GitHub named, or `null` before the first answer. */
  newest: string | null
  /** Whether `newest` is later than `current`. */
  available: boolean
  release_url: string | null
  checked_at: number | null
  /** Why the last look at GitHub failed; `newest` is then the one seen before. */
  error: string | null
  /** Whether the WSL package runs `thoric update` when the dashboard asks. */
  installable: boolean
  /** The running update job, which outlives the manager it replaces. */
  job_id: string | null
}

/**
 * One setting of `GET configuration`, named as in `.env`, with the value the manager runs with. A
 * secret comes without its value, which never leaves the manager. A setting the Settings page may
 * change has `editable`, and unless it is a secret, its `text`, the value as the manager's
 * environment gave it. The page replaces or removes an editable secret without reading it.
 */
export type ConfigVariable =
  | {
      name: string
      value: string | number | boolean | null
      secret?: undefined
      editable?: boolean
      text?: string
    }
  | { name: string; secret: true; set: boolean; editable?: boolean }

/** `GET configuration`: the installation's settings, in the order the settings page lists them. */
export type Configuration = {
  variables: ConfigVariable[]
  /** Whether the WSL package takes settings changes from this page. */
  editable?: boolean
  /** The settings change that is running, which outlives the manager it restarts. */
  job_id?: string | null
  /** What answers without a credential while Docker publishes the manager beyond loopback. */
  exposure?: { open: ("dashboard" | "mcp")[]; published: string[] } | null
}

/** How an address of `NOTIFY_URL` is sent to. */
export type NotifyFormat = "ntfy" | "discord" | "slack" | "telegram" | "json"

/**
 * `POST settings/notify/test`: how each address of `NOTIFY_URL` took the test notice, named by
 * its format and host alone, since the rest of it can hold a credential.
 */
export type NotifyTest = {
  results: {
    format: NotifyFormat
    host: string
    ok: boolean
    status: number | null
    answer: string
    error: string | null
  }[]
}

/** `POST configuration` with `dry_run`: what is wrong with a change, before anything restarts. */
export type ConfigurationCheck = {
  errors: Record<string, string>
  error: string | null
  running_jobs: number
}

export type SystemInfo = {
  connected: boolean
  version?: string
  cpus?: number
  memory_bytes?: number
  containers?: number
  error?: string
}

export type Session = { auth_enabled: boolean; authenticated: boolean }

/** An agent's request for a person at its desktop, such as for a CAPTCHA or a sign-in. */
export type HumanRequest = {
  id: string
  container_id: string
  /** The container's name when the agent asked. */
  container_name: string
  message: string
  /** The MCP endpoint the agent asked through; `null` for `/mcp`. */
  endpoint: string | null
  status: "pending" | "done" | "declined"
  reply: string | null
  created_at: number
  answered_at: number | null
}

export type JobStarted = { job_id: string }

export type SettingsPreview = { container_id: string; live: string[]; restart: string[] }

/** What an MCP endpoint lets an agent do in one container, beside its desktop MCP servers. */
export type McpScope =
  "view" | "input" | "applications" | "files_read" | "files_write" | "commands" | "media" | "power"

export type McpTool = { name: string; description: string }

/** A desktop server's tool, with `exposed`, its name on an endpoint of its container alone. */
export type ServerTool = McpTool & { exposed: string }

/**
 * An MCP server running inside a container: declared by its image (`image`), or registered in
 * the running desktop (`container`). Environment variables and headers are listed by name only.
 */
export type DesktopServer = {
  container_id: string
  container_name: string
  name: string
  source: "image" | "container"
  description: string
  transport: "stdio" | "http"
  command: string | null
  args: string[]
  cwd: string | null
  env: string[]
  url: string | null
  headers: string[]
  /** `pending` before its tools were read, `stopped` while the container is not running. */
  state: "ready" | "error" | "pending" | "stopped"
  error: string | null
  tools: ServerTool[]
  /** Unix seconds. */
  tools_at: number | null
  /** The end of the server's standard error, which may be empty. */
  log: string
}

/**
 * A desktop MCP server to register, or one an image declares: a command, or the URL of a server
 * that listens inside the desktop. Values may name compose values as `${NAME}`.
 */
export type ServerRequest = {
  name: string
  description?: string
  command?: string
  args?: string[]
  env?: Record<string, string>
  cwd?: string
  url?: string
  headers?: Record<string, string>
}

/** A desktop MCP server a build writes into its image. */
export type ServerDeclaration = ServerRequest

/** One container an MCP endpoint reaches, and what it may do there. */
export type McpGrant = {
  container_id: string
  container_name: string
  alias: string
  scopes: McpScope[]
  /** The desktop MCP servers whose tools it includes: all, also later ones, or these. */
  servers: "all" | string[]
  /**
   * Tools left out although their scope or server is included, by the name the endpoint gives
   * them for this container alone: a container tool's own name, or `SERVER__TOOL`. Empty means
   * everything the scopes and servers give.
   */
  excluded_tools: string[]
}

export type GrantRequest = Omit<McpGrant, "container_name" | "alias" | "excluded_tools"> & {
  alias?: string
  excluded_tools?: string[]
}

/** A scoped MCP door at `/mcp/<name>` with a bearer token of its own. */
export type McpEndpoint = {
  name: string
  /** `/mcp/<name>`, relative to the dashboard's origin. */
  path: string
  /** Created with its container. */
  automatic: boolean
  has_token: boolean
  token_created_at: number | null
  created_at: number
  grants: McpGrant[]
  tool_count: number
}

/** A token is returned once, when it is created or replaced. */
export type EndpointToken = { endpoint: McpEndpoint; token: string }

/** A tool as an endpoint exposes it; `container_id` is null for a tool of the endpoint itself, such as `get_job`. */
export type EndpointTool = McpTool & {
  source: "thoric" | "server"
  container_id: string | null
  server: string | null
}

/** The scopes and the Thoric tools in each; `default` is what a new grant starts with. */
export type McpScopes = { scopes: { id: McpScope; tools: string[] }[]; default: McpScope[] }
