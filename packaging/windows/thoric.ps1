# Thoric's Windows helper: installs the WSL distribution, then starts, stops, updates, moves,
# resizes and removes it, and opens it to other devices. It runs straight from a release, without
# saving it:
#
#   irm https://github.com/co3moz/thoric/releases/latest/download/thoric.ps1 | iex
#
# 'irm | iex' runs this text in the caller's scope, so everything lives inside Invoke-Thoric and
# only that function stays behind in the PowerShell window. Keep the file ASCII, since Windows
# PowerShell reads a script without a byte order mark in the ANSI code page, and never call 'exit',
# which would close the window that ran it.
#
# Copyright (C) 2026 Thoric contributors. Thoric is free software under the GNU General Public
# License, version 3 or later, and comes with no warranty: https://github.com/co3moz/thoric/blob/master/LICENSE
function Invoke-Thoric {
    param(
        [Parameter(Position = 0)][string]$Command = "",
        [Parameter(Position = 1)][string]$Option = "",
        [string]$Package = "",
        [string]$Location = "",
        [string]$VhdSize = "",
        [string]$Distro = "Thoric",
        [int]$Port = 0,
        [string]$Address = "",
        [string]$TaskUser = "",
        [string]$Log = ""
    )
    # -Address, -TaskUser and -Log are what 'lan' hands the copy of this script that runs as administrator.
    $ErrorActionPreference = "Stop"
    $taskName = "Thoric keep-alive"
    # What the auto-start task runs in the distribution: 'thoric start' first, since a manager that 'stop'
    # or 'thoric stop' stopped stays down under Compose's 'restart: unless-stopped', then the keep-alive
    # process. The output of 'thoric start' goes to the journal, 'journalctl -t thoric'.
    $autoStart = "thoric start 2>&1 | logger -t thoric; exec /usr/lib/wsl/keepalive.sh"
    # Under NAT, a task with administrator rights points the port proxy at the address in this file, which
    # 'start' and the keep-alive task write without them.
    $lanTaskName = "Thoric LAN"
    $addressFile = Join-Path $env:LOCALAPPDATA "Thoric\wsl-address"
    $releaseUrl = "https://github.com/co3moz/thoric/releases/latest/download"
    $helperUrl = "$releaseUrl/thoric.ps1"
    # The first WSL that installs a .wsl package.
    $minimumWsl = [version]"2.4.4"
    # WSL's settings for its virtual machine, which every distribution shares.
    $wslConfigFile = Join-Path $env:USERPROFILE ".wslconfig"
    $usage = @"
Usage: thoric.ps1 [command] [options]

Without a command it offers to install Thoric, or lists what it can do with an installed one.

  install                             Install or update WSL when needed, then download the latest package,
                                      register it, run its first-start setup and start it
      -Location D:\WSL\Thoric     Keep the distribution's disk in this folder
      -VhdSize 200GB                  Set the disk's maximum size (1 TB unless given)
      -Package .\thoric.wsl       Install this package file instead of downloading one
  start                               Boot the distribution, keep it running and start the manager
  stop                                Stop the manager and shut the distribution down
  status                              Show the manager state
  open                                Open the dashboard in the browser
  update                              Move the installation to the newest release
  move D:\WSL\Thoric              Move the distribution's disk to another folder
  resize 200GB                        Change the disk's maximum size
  autostart enable|disable            Start Thoric at Windows sign-in through a scheduled task
  lan enable|disable|status           Let other devices reach the dashboard through Windows
  wslconfig                           Show the memory, processors and swap of WSL's virtual machine, and offer
                                      values for those .wslconfig leaves to WSL
  uninstall                           Remove the auto-start task and unregister the distribution

  -Distro <name>                      Work on a distribution installed under another name
  -Port <port>                        Use this dashboard port instead of the one in .env

A command runs without saving the helper too:
  & ([scriptblock]::Create((irm $helperUrl))) move D:\WSL\Thoric

'install' needs WSL $minimumWsl or newer on the Virtual Machine Platform. It offers 'wsl --install
--no-distribution' where WSL or that feature is missing and 'wsl --update' for an older WSL, then
values for .wslconfig, which WSL reads when its virtual machine starts. A newly enabled feature
needs a restart of Windows before the package installs. It downloads the package of Windows'
architecture: thoric.wsl on x64 Windows, thoric-arm64.wsl on Windows on ARM.

WSL stops a distribution shortly after its last Windows-started process ends, so 'start' and the
auto-start task keep a hidden keep-alive process open while Docker and the manager run inside. Both
run 'thoric start' too, which brings back a manager that 'stop' stopped.

WSL lets go of a stopped distribution's disk only when its virtual machine shuts down, so 'move'
and 'resize' shut WSL down, asking first when other distributions are running, and then start
Thoric again.

'lan enable' opens the dashboard's port, or -Port, through Windows once BIND_ADDRESS in the
distribution's .env lets the manager listen beyond localhost. It asks for administrator rights and sets
up what the WSL networking mode needs: a Hyper-V firewall rule under mirrored networking, a port proxy
to the distribution with a Windows firewall rule under NAT. The distribution gets a new NAT address
whenever WSL restarts, and 'start' and the auto-start task point the port proxy at it.
"@

    function Invoke-Wsl { param([string[]]$Arguments) & wsl.exe @Arguments; if ($LASTEXITCODE -ne 0) { throw "wsl.exe exited with code $LASTEXITCODE" } }
    # The version of the WSL package, from the wsl.exe it installs, or nothing without one. Windows' own
    # wsl.exe is all there is then, and it answers every command with a note on stderr that WSL is not
    # installed, so nothing here asks wsl.exe about distributions before this finds the package.
    function Get-WslVersion {
        $folder = (Get-ItemProperty HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Lxss\MSI -ErrorAction SilentlyContinue).InstallLocation
        if (-not $folder) { $folder = Join-Path $env:ProgramFiles "WSL" }
        $file = Get-Item -LiteralPath (Join-Path $folder "wsl.exe") -ErrorAction SilentlyContinue
        if ($file -and $file.VersionInfo.FileVersion -match '^[0-9]+(\.[0-9]+){1,3}') { return [version]$Matches[0] }
    }
    # The architecture of Windows itself, AMD64 or ARM64. On Windows on ARM a process that runs under
    # emulation, such as an x64 PowerShell 7, finds AMD64 or x86 in its own PROCESSOR_ARCHITECTURE, so
    # the machine's value in the registry comes first.
    function Get-WindowsArchitecture {
        $machine = (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" -ErrorAction SilentlyContinue).PROCESSOR_ARCHITECTURE
        if ($machine) { return $machine }
        if ($env:PROCESSOR_ARCHITEW6432) { return $env:PROCESSOR_ARCHITEW6432 }
        return $env:PROCESSOR_ARCHITECTURE
    }
    # The release asset with the package for an architecture of Windows, which WSL 2 runs on x64 and ARM64.
    function Get-PackageName {
        param([string]$Architecture)
        switch ($Architecture) {
            "AMD64" { return "thoric.wsl" }
            "ARM64" { return "thoric-arm64.wsl" }
        }
        throw "WSL 2 runs on x64 and ARM64 Windows, not on $Architecture."
    }
    function Test-Registered {
        if (-not (Get-WslVersion)) { return $false }
        $list = (& wsl.exe --list --quiet) -join "`n"
        return ($list -replace "`0", "") -match "(?m)^$Distro\s*$"
    }
    # Whether the Virtual Machine Platform, the Windows feature WSL 2 runs on, is on; nothing when Windows
    # does not say.
    function Test-VirtualMachinePlatform {
        $feature = Get-CimInstance Win32_OptionalFeature -Filter "Name = 'VirtualMachinePlatform'" -ErrorAction SilentlyContinue
        if ($feature) { return $feature.InstallState -eq 1 }
    }
    # Runs wsl.exe in this window without taking its output, so a function can call it and still return a
    # value, and returns its exit code.
    function Start-Wsl {
        param([string[]]$Arguments)
        return (Start-Process -FilePath "wsl.exe" -ArgumentList $Arguments -NoNewWindow -Wait -PassThru).ExitCode
    }
    # Whether WSL can install the package: WSL 2.4.4 or newer on the Virtual Machine Platform, with the
    # Windows hypervisor running. It offers what is missing, 'wsl --install --no-distribution' for WSL or
    # the feature and 'wsl --update' for an older WSL; both ask for administrator rights themselves, and a
    # feature they enable runs only after a restart.
    function Install-Wsl {
        $version = Get-WslVersion
        $arguments = @()
        if (-not $version) {
            Write-Host "Thoric runs in WSL 2, which is not installed."
            $arguments = @("--install", "--no-distribution")
        } elseif ((Test-VirtualMachinePlatform) -eq $false) {
            Write-Host "WSL $version is installed, but the Virtual Machine Platform feature it runs on is off."
            $arguments = @("--install", "--no-distribution")
        } elseif ($version -lt $minimumWsl) {
            Write-Host "WSL $version is installed; Thoric's package needs WSL $minimumWsl or newer."
            $arguments = @("--update")
        }
        if ($arguments) {
            Write-Host "'wsl $($arguments -join ' ')' sets it up. Windows asks for administrator rights, and a feature it turns on needs a restart."
            if ((Read-Host "Run it now? Enter for yes, 'no' to stop") -notin "", "y", "yes") { Write-Host "Nothing was changed."; return $false }
            $code = Start-Wsl $arguments
            # A WSL from the Store, before its MSI package, has no version here; 'wsl --update' replaces it.
            if (-not (Get-WslVersion) -or (Get-WslVersion) -lt $minimumWsl) { $code = Start-Wsl @("--update") }
            $version = Get-WslVersion
            if (-not $version -or $version -lt $minimumWsl) {
                throw "WSL $minimumWsl or newer is still not installed (wsl.exe exited with code $code). Restart Windows if WSL asked for it and run the helper again, or install WSL as https://aka.ms/wslinstall describes."
            }
        }
        $computer = Get-CimInstance Win32_ComputerSystem -ErrorAction SilentlyContinue
        if ((Test-VirtualMachinePlatform) -eq $false -or ($computer -and -not $computer.HypervisorPresent)) {
            Write-Host "WSL 2 runs on the Windows hypervisor, which is not running yet. Restart Windows to finish turning on the Virtual Machine Platform, then run the helper again."
            Write-Host "If it still does not run after a restart, turn on virtualization (Intel VT-x or AMD-V) in the computer's firmware settings."
            return $false
        }
        return $true
    }

    # The settings of WSL's virtual machine for a computer with this much memory, in GB, and this many
    # processors. Windows keeps a quarter of the memory, at least 4 GB, and of more than four processors a
    # quarter, at least two. Swap is a quarter of the memory, as WSL's own, but at most 8 GB, since it is a
    # file on the Windows drive.
    function Get-WslSuggestion {
        param([int]$MemoryGB, [int]$Processors)
        $memory = [int][math]::Max(2, $MemoryGB - [math]::Max(4, [math]::Floor($MemoryGB / 4)))
        $cpus = if ($Processors -gt 4) { $Processors - [int][math]::Max(2, [math]::Floor($Processors / 4)) } else { $Processors }
        $swap = [int][math]::Min(8, [math]::Ceiling($memory / 4))
        return [ordered]@{ memory = "${memory}GB"; processors = "$cpus"; swap = "${swap}GB" }
    }
    # What the [wsl2] section of .wslconfig's lines sets, by key.
    function Read-WslConfig {
        param([string[]]$Lines)
        $settings = @{}
        $section = ""
        foreach ($line in $Lines) {
            if ($line -match '^\s*\[([^\]]*)\]\s*$') { $section = $Matches[1].Trim().ToLower() }
            elseif ($section -eq "wsl2" -and $line -match '^\s*([A-Za-z0-9_.]+)\s*=\s*(.*?)\s*$') { $settings[$Matches[1].ToLower()] = $Matches[2] }
        }
        return $settings
    }
    # .wslconfig's lines with these settings first in its [wsl2] section, or in a [wsl2] section added at the
    # end, and every other line as it was.
    function Add-WslSettings {
        param([string[]]$Lines, [Collections.IDictionary]$Settings)
        $added = [string[]]@($Settings.Keys | ForEach-Object { "$_=$($Settings[$_])" })
        $result = New-Object Collections.Generic.List[string]
        $done = $false
        foreach ($line in $Lines) {
            $result.Add($line)
            if (-not $done -and $line -match '^\s*\[\s*wsl2\s*\]\s*$') { $result.AddRange($added); $done = $true }
        }
        if (-not $done) {
            while ($result.Count -and -not $result[$result.Count - 1].Trim()) { $result.RemoveAt($result.Count - 1) }
            if ($result.Count) { $result.Add("") }
            $result.Add("[wsl2]")
            $result.AddRange($added)
        }
        return , $result.ToArray()
    }
    # The memory, processors and swap of WSL's virtual machine, which Thoric's desktops share with every
    # other distribution. Shows what .wslconfig sets and offers values for what it leaves to WSL, which
    # gives the machine half of the memory, every processor and a quarter of its memory as swap. They go
    # into the file's [wsl2] section, in the file's own encoding and line endings, and nothing else in it
    # changes.
    function Confirm-WslConfig {
        $lines = @()
        $text = ""
        $encoding = New-Object Text.UTF8Encoding $false
        if (Test-Path -LiteralPath $wslConfigFile) {
            $bytes = [IO.File]::ReadAllBytes($wslConfigFile)
            if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) { $encoding = [Text.Encoding]::Unicode }
            elseif ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { $encoding = New-Object Text.UTF8Encoding $true }
            $text = [IO.File]::ReadAllText($wslConfigFile, $encoding)
            $lines = @($text.TrimEnd("`r", "`n") -split "\r?\n")
        }
        $current = Read-WslConfig $lines
        $computer = Get-CimInstance Win32_ComputerSystem
        $memoryGB = [int][math]::Round($computer.TotalPhysicalMemory / 1GB)
        $suggested = Get-WslSuggestion $memoryGB $computer.NumberOfLogicalProcessors
        $set = @($suggested.Keys | Where-Object { $current.ContainsKey($_) } | ForEach-Object { "$_=$($current[$_])" })
        if ($set) { Write-Host "WSL's virtual machine gets $($set -join ', ') from $wslConfigFile." }
        $missing = [ordered]@{}
        foreach ($key in $suggested.Keys) { if (-not $current.ContainsKey($key)) { $missing[$key] = $suggested[$key] } }
        if (-not $missing.Count) { return }
        $defaults = @{
            memory = "half of this computer's $memoryGB GB of memory"
            processors = "all $($computer.NumberOfLogicalProcessors) processors"
            swap = "a quarter of its memory as swap"
        }
        $parts = @($missing.Keys | ForEach-Object { $defaults[$_] })
        $said = if ($parts.Count -gt 1) { ($parts[0..($parts.Count - 2)] -join ", ") + " and " + $parts[-1] } else { $parts[0] }
        Write-Host "WSL gives its virtual machine, which Thoric's desktops share with every other distribution, $said unless .wslconfig says otherwise. Suggested:"
        foreach ($key in $missing.Keys) { Write-Host "  $key=$($missing[$key])" }
        if ((Read-Host "Add them to ${wslConfigFile}? Enter for yes, 'no' to leave them to WSL") -notin "", "y", "yes") { return }
        $newline = if ($text -and $text -notmatch "`r`n") { "`n" } else { "`r`n" }
        [IO.File]::WriteAllText($wslConfigFile, (((Add-WslSettings $lines $missing) -join $newline) + $newline), $encoding)
        Write-Host "Added them to $wslConfigFile."
        if ((Get-WslVersion) -and (Get-Running)) {
            Write-Host "WSL reads it when its virtual machine starts. Run 'wsl --shutdown', which stops every distribution, then 'start'."
        }
    }
    # A list that fails counts as nothing running.
    function Get-Running {
        $list = & wsl.exe --list --running --quiet
        if ($LASTEXITCODE -ne 0) { return @() }
        return @($list | ForEach-Object { ($_ -replace "`0", "").Trim() } | Where-Object { $_ })
    }
    function Get-DiskFolder {
        $keys = Get-ChildItem HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss -ErrorAction SilentlyContinue
        return ($keys | ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.DistributionName -eq $Distro }).BasePath
    }
    # wsl.exe resolves a relative path against its own working folder, so it gets a full one.
    function Resolve-Folder { param([string]$Path) return $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path) }
    # The settings these commands read from the installation's .env, without the quotes the first start puts
    # around some values. The secrets in the same file stay inside the distribution.
    function Get-Settings {
        $settings = @{}
        $lines = & wsl.exe -d $Distro -- sh -c "grep -E '^(PORT|BIND_ADDRESS|AUTH_MODE|ROOT_MCP_ENABLED|MCP_ENABLED|MCP_AUTH_MODE)=' /opt/thoric/.env 2>/dev/null"
        foreach ($line in $lines) {
            if (($line -replace "`0", "") -match '^([A-Z_]+)=(.*)$') { $settings[$Matches[1]] = $Matches[2].Trim('"', "'", " ") }
        }
        return $settings
    }
    # The port is a setting of the installation; -Port overrides what its .env says.
    function Get-Port {
        param([hashtable]$Settings)
        if ($Port) { return $Port }
        if (-not $Settings) { $Settings = Get-Settings }
        if ($Settings["PORT"] -match '^[0-9]+$') { return [int]$Settings["PORT"] }
        return 8080
    }
    function Test-Loopback { param([string]$Bind) return ($Bind -like "127.*" -or $Bind -eq "localhost") }
    # Matches this distribution's keep-alive only, as started here or by the auto-start task of this
    # helper or an earlier one. It starts the keep-alive process alone, since 'start' runs 'thoric start'
    # in this window right after.
    function Start-KeepAlive {
        $running = Get-CimInstance Win32_Process -Filter "Name = 'wsl.exe'" | Where-Object { $_.CommandLine -like "*-d $Distro -- *keepalive.sh*" }
        if (-not $running) {
            Start-Process -FilePath "wsl.exe" -ArgumentList @("-d", $Distro, "--", "/usr/lib/wsl/keepalive.sh") -WindowStyle Hidden | Out-Null
        }
    }
    # The auto-start task. With LAN access under NAT it first reads the distribution's new address, without
    # administrator rights, and has the LAN task point the port proxy at it.
    function Register-KeepAlive {
        $actions = @()
        if (Get-ScheduledTask -TaskName $lanTaskName -ErrorAction SilentlyContinue) {
            $file = $addressFile -replace "'", "''"
            $lookup = "for (`$i = 0; `$i -lt 30; `$i++) { `$line = (wsl.exe -d $Distro --exec ip -4 -o addr show dev eth0) -join ' '; " +
                "if (`$line -match 'inet ([0-9.]+)/') { Set-Content -LiteralPath '$file' -Value `$Matches[1]; Start-ScheduledTask -TaskName '$lanTaskName'; break }; Start-Sleep -Seconds 2 }"
            $actions += New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -Command `"$lookup`""
        }
        $actions += New-ScheduledTaskAction -Execute "wsl.exe" -Argument "-d $Distro -- sh -c `"$autoStart`""
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
        $settings = New-ScheduledTaskSettingsSet -Hidden -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName $taskName -Action $actions -Trigger $trigger -Settings $settings -Force | Out-Null
    }
    # A task an earlier helper registered runs only the keep-alive process, so a stopped manager stayed
    # down; registering it again brings 'thoric start'.
    function Update-KeepAlive {
        $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        if ($task -and -not ($task.Actions | Where-Object { $_.Arguments -like "*thoric start*" })) { Register-KeepAlive }
    }
    # Whether the first start finished: it writes the settings last.
    function Test-Settings {
        & wsl.exe -d $Distro -- test -f /opt/thoric/.env
        return $LASTEXITCODE -eq 0
    }
    function Start-Thoric {
        Update-KeepAlive
        Start-KeepAlive
        Invoke-Wsl @("-d", $Distro, "--", "thoric", "start")
        try { Update-PortProxy } catch { Write-Warning "Other devices may not reach Thoric: $($_.Exception.Message)" }
        Write-Host "Dashboard: http://localhost:$(Get-Port)"
    }
    # A terminated distribution's disk stays attached to the WSL virtual machine, and WSL refuses to
    # move or resize it until the machine shuts down, which stops every other distribution too.
    function Invoke-DiskChange {
        param([string[]]$Arguments, [string]$Message)
        if (-not (Test-Registered)) { throw "The $Distro distribution is not registered. Run 'install' first." }
        $running = Get-Running
        $others = @($running | Where-Object { $_ -ne $Distro })
        if ($others) {
            Write-Host "This shuts WSL down, which also stops $($others -join ', ')."
            if ((Read-Host "Type 'yes' to continue") -ne "yes") { Write-Host "Nothing was changed."; return }
        }
        if ($running -contains $Distro) { & wsl.exe -d $Distro -- thoric stop }
        Invoke-Wsl @("--shutdown")
        Write-Host $Message
        try { Invoke-Wsl $Arguments } finally { Start-Thoric }
    }

    # The WSL networking mode, a Windows setting shared by every distribution: nat, mirrored or another.
    function Get-NetworkingMode { return ((& wsl.exe -d $Distro --exec wslinfo --networking-mode) -join "").Trim() }
    # The distribution's address on the WSL network under NAT, which changes whenever WSL restarts.
    function Get-DistroAddress {
        $line = (& wsl.exe -d $Distro --exec ip -4 -o addr show dev eth0) -join " "
        if ($line -match 'inet ([0-9.]+)/') { return $Matches[1] }
        throw "Could not read the address of the $Distro distribution."
    }
    function Save-DistroAddress {
        param([string]$Value)
        New-Item -ItemType Directory -Force -Path (Split-Path $addressFile) | Out-Null
        Set-Content -LiteralPath $addressFile -Value $Value -Encoding ASCII
    }
    # Where the port proxy of 'lan enable' sends a port, as netsh lists it, or nothing.
    function Get-PortProxy {
        param([int]$ListenPort)
        foreach ($line in (& netsh.exe interface portproxy show v4tov4)) {
            if ($line -match "^\s*0\.0\.0\.0\s+$ListenPort\s+(\S+)\s+[0-9]+\s*$") { return $Matches[1] }
        }
    }
    # The port a firewall rule of 'lan enable' lets in: the Windows firewall's under NAT, the Hyper-V
    # firewall's under mirrored networking.
    function Get-RulePort {
        param([switch]$HyperV)
        if ($HyperV) {
            if (-not (Get-Command Get-NetFirewallHyperVRule -ErrorAction SilentlyContinue)) { return }
            $rule = Get-NetFirewallHyperVRule -Name Thoric -ErrorAction SilentlyContinue
            if ($rule) { $first = @($rule.LocalPorts)[0] }
        } else {
            $rule = Get-NetFirewallRule -Name Thoric -ErrorAction SilentlyContinue
            if ($rule) { $first = @(($rule | Get-NetFirewallPortFilter).LocalPort)[0] }
        }
        if ($rule -and $first -match '^[0-9]+$') { return [int]$first }
    }
    function Test-LanSetUp {
        param([int]$ListenPort)
        return [bool]((Get-RulePort) -or (Get-RulePort -HyperV) -or (Get-PortProxy $ListenPort) -or (Get-ScheduledTask -TaskName $lanTaskName -ErrorAction SilentlyContinue))
    }
    # The addresses other devices use for this computer: those of the connected adapters with a gateway.
    function Get-WindowsAddresses {
        $addresses = @(Get-NetIPConfiguration | Where-Object { $_.IPv4DefaultGateway -and $_.NetAdapter.Status -eq "Up" } | ForEach-Object { $_.IPv4Address.IPAddress })
        if (-not $addresses) { $addresses = @("<this computer's address>") }
        return $addresses
    }

    function Test-Administrator {
        $principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
        return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    }
    # A task with the highest run level gets administrator rights only for an account in the Administrators
    # group, which a PowerShell without them still lists, as a group used for deny only.
    function Test-AdministratorAccount { return [bool]((& whoami.exe /groups /fo csv /nh) -match '"S-1-5-32-544"') }

    # The Windows side of LAN access, as administrator. It removes what an earlier 'lan enable' set up, then
    # opens the port for nat or mirrored networking, or nothing for none. Under NAT it adds the port proxy to
    # the distribution's address, the Windows firewall rule and, for an administrator's account, the LAN
    # task. That task accepts only an IPv4 address from the address file, which the account can write
    # without administrator rights.
    function Set-LanAccess {
        param([string]$Mode, [int]$ListenPort, [string]$Target, [string]$User)
        $ports = @($ListenPort)
        $rule = Get-NetFirewallRule -Name Thoric -ErrorAction SilentlyContinue
        if ($rule) { $ports += @(($rule | Get-NetFirewallPortFilter).LocalPort) }
        foreach ($old in $ports | Select-Object -Unique) { netsh.exe interface portproxy delete v4tov4 listenport=$old listenaddress=0.0.0.0 | Out-Null }
        Remove-NetFirewallRule -Name Thoric -ErrorAction SilentlyContinue
        $hyperV = [bool](Get-Command New-NetFirewallHyperVRule -ErrorAction SilentlyContinue)
        if ($hyperV) { Remove-NetFirewallHyperVRule -Name Thoric -ErrorAction SilentlyContinue }
        Unregister-ScheduledTask -TaskName $lanTaskName -Confirm:$false -ErrorAction SilentlyContinue
        if ($Mode -eq "mirrored") {
            if (-not $hyperV) { throw "Mirrored networking needs the Hyper-V firewall, which this Windows lacks." }
            New-NetFirewallHyperVRule -Name Thoric -DisplayName Thoric -Direction Inbound -VMCreatorId "{40E0AC32-46A5-438A-A0B2-2B479E8F2E90}" -Protocol TCP -LocalPorts $ListenPort | Out-Null
        } elseif ($Mode -eq "nat") {
            netsh.exe interface portproxy add v4tov4 listenport=$ListenPort listenaddress=0.0.0.0 connectport=$ListenPort connectaddress=$Target | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "netsh could not forward port $ListenPort to $Target." }
            New-NetFirewallRule -Name Thoric -DisplayName Thoric -Direction Inbound -Protocol TCP -LocalPort $ListenPort -Action Allow | Out-Null
            if ($User) {
                $file = $addressFile -replace "'", "''"
                $refresh = "`$address = Get-Content -LiteralPath '$file' -TotalCount 1; if (`$address -match '^[0-9]{1,3}(\.[0-9]{1,3}){3}`$') { " +
                    "netsh.exe interface portproxy set v4tov4 listenport=$ListenPort listenaddress=0.0.0.0 connectport=$ListenPort connectaddress=`$address }"
                $action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -Command `"$refresh`""
                $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Highest
                $settings = New-ScheduledTaskSettingsSet -Hidden -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 5)
                Register-ScheduledTask -TaskName $lanTaskName -Action $action -Principal $principal -Settings $settings -Force | Out-Null
            }
        }
    }
    # Points the port proxy at the distribution's new address, as administrator.
    function Set-PortProxy {
        param([int]$ListenPort, [string]$Target)
        netsh.exe interface portproxy set v4tov4 listenport=$ListenPort listenaddress=0.0.0.0 connectport=$ListenPort connectaddress=$Target | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "netsh could not forward port $ListenPort to $Target." }
    }
    # One administrator step of 'lan': nat, mirrored or none set LAN access up for that mode or remove it,
    # proxy points the port proxy at a new address. None of them runs anything from the distribution.
    function Invoke-AdministratorStep {
        param([string]$Step, [int]$ListenPort, [string]$Target, [string]$User)
        if ($Step -eq "proxy") { Set-PortProxy $ListenPort $Target }
        elseif ($Step -in "nat", "mirrored", "none") { Set-LanAccess $Step $ListenPort $Target $User }
        else { throw "Unknown administrator step: $Step" }
    }
    # Runs an administrator step, through a UAC prompt unless this PowerShell already has the rights. The
    # elevated copy of this script gets the step and its values on its command line, which the UAC prompt
    # shows. It has a window of its own, so its error comes back in a file. Run through 'irm | iex', the
    # helper has no file of its own, so the elevated copy runs from this function written beside that log.
    function Invoke-Elevated {
        param([string]$Step, [int]$ListenPort, [string]$Target = "", [string]$User = "")
        if (Test-Administrator) { Invoke-AdministratorStep $Step $ListenPort $Target $User; return }
        $log = [IO.Path]::GetTempFileName()
        $script = $PSCommandPath
        $copy = ""
        if (-not $script) {
            $copy = [IO.Path]::ChangeExtension($log, ".ps1")
            Set-Content -LiteralPath $copy -Encoding ASCII -Value ("function Invoke-Thoric {" + ${function:Invoke-Thoric}.ToString() + "}`nInvoke-Thoric @args")
            $script = $copy
        }
        $arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$script`" elevated $Step -Port $ListenPort -Log `"$log`""
        if ($Target) { $arguments += " -Address $Target" }
        if ($User) { $arguments += " -TaskUser `"$User`"" }
        try {
            $process = Start-Process -FilePath "powershell.exe" -ArgumentList $arguments -Verb RunAs -WindowStyle Hidden -Wait -PassThru
        } catch {
            Remove-Item -LiteralPath $log -ErrorAction SilentlyContinue
            throw "This step needs administrator rights, and the UAC prompt was declined."
        } finally {
            if ($copy) { Remove-Item -LiteralPath $copy -ErrorAction SilentlyContinue }
        }
        $failure = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $log -ErrorAction SilentlyContinue
        if ($failure) { throw $failure.Trim() }
        if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) { throw "The administrator step exited with code $($process.ExitCode)." }
    }

    # After WSL restarts, the port proxy of 'lan enable' sends the port to the distribution's old NAT address.
    # The LAN task points it at the new one; without that task, UAC asks.
    function Update-PortProxy {
        $lanPort = Get-RulePort
        if (-not $lanPort) { return }
        $target = Get-PortProxy $lanPort
        if (-not $target) { return }
        if ((Get-NetworkingMode) -ne "nat") {
            Write-Warning "Port $lanPort still has the port proxy of NAT networking, which WSL no longer uses. Run 'lan enable' again."
            return
        }
        $address = Get-DistroAddress
        if ($target -eq $address) { return }
        Save-DistroAddress $address
        if (Get-ScheduledTask -TaskName $lanTaskName -ErrorAction SilentlyContinue) {
            try { Start-ScheduledTask -TaskName $lanTaskName } catch { }
            for ($i = 0; $i -lt 20 -and (Get-PortProxy $lanPort) -ne $address; $i++) { Start-Sleep -Milliseconds 500 }
        }
        if ((Get-PortProxy $lanPort) -ne $address) {
            Write-Host "$Distro has a new address, $address. Windows asks for administrator rights to forward port $lanPort to it."
            Invoke-Elevated "proxy" $lanPort $address
        }
        Write-Host "Port $lanPort reaches $Distro at its new address, $address."
    }

    function Enable-Lan {
        if (-not (Test-Registered)) { throw "The $Distro distribution is not registered. Run 'install' first." }
        $settings = Get-Settings
        $lanPort = Get-Port $settings
        $bind = $settings["BIND_ADDRESS"]
        if (-not $bind) { $bind = "127.0.0.1" }
        if (Test-Loopback $bind) {
            throw "The manager listens on ${bind}:$lanPort, which only this computer reaches, whatever Windows allows. Set BIND_ADDRESS=0.0.0.0 with 'thoric settings' in the distribution, run 'thoric restart', then 'lan enable' again."
        }
        if ($settings["AUTH_MODE"] -eq "none") {
            Write-Warning "The dashboard has no password (AUTH_MODE=none), so any device that reaches it controls every container."
        }
        # ROOT_MCP_ENABLED was called MCP_ENABLED before 0.6.3, and the manager still reads the old name
        # while the new one is missing. MCP endpoints made on the dashboard answer either way.
        $rootMcp = $settings["ROOT_MCP_ENABLED"]
        if (-not $rootMcp) { $rootMcp = $settings["MCP_ENABLED"] }
        if ($settings["MCP_AUTH_MODE"] -eq "none") {
            if ($rootMcp -eq "true") {
                Write-Warning "Agents need no token (MCP_AUTH_MODE=none), so any device that reaches the agent endpoint controls every container."
            } else {
                Write-Warning "MCP endpoints need no token (MCP_AUTH_MODE=none), so any device that reaches one controls the containers it grants."
            }
        }
        $mode = Get-NetworkingMode
        $address = ""
        $taskUser = ""
        if ($mode -eq "nat") {
            $address = Get-DistroAddress
            Save-DistroAddress $address
            if (Test-AdministratorAccount) { $taskUser = [Security.Principal.WindowsIdentity]::GetCurrent().Name }
        } elseif ($mode -ne "mirrored") {
            throw "WSL uses '$mode' networking; 'lan' handles NAT and mirrored networking only."
        }
        Write-Host "Opening port $lanPort for other devices under $mode networking. Windows asks for administrator rights."
        Invoke-Elevated $mode $lanPort $address $taskUser
        if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) { Register-KeepAlive }
        if ($mode -eq "mirrored") {
            Write-Host "The Hyper-V firewall lets port $lanPort in."
        } else {
            Write-Host "Windows forwards port $lanPort to $Distro at $address, and its firewall lets the port in."
            if ($taskUser) {
                Write-Host "That address changes whenever WSL restarts; 'start' and auto-start then point the port proxy at the new one through the '$lanTaskName' task."
            } else {
                Write-Host "That address changes whenever WSL restarts; 'start' then asks for administrator rights to point the port proxy at the new one."
            }
        }
        foreach ($ip in Get-WindowsAddresses) { Write-Host "Other devices open http://${ip}:$lanPort" }
    }

    function Disable-Lan {
        $lanPort = if ($Port) { $Port } elseif (Test-Registered) { Get-Port } else { 8080 }
        if (-not (Test-LanSetUp $lanPort)) {
            Write-Host "Windows has nothing set up for other devices."
            return
        }
        Write-Host "Closing the way in for other devices. Windows asks for administrator rights."
        Invoke-Elevated "none" $lanPort
        Remove-Item -LiteralPath $addressFile -ErrorAction SilentlyContinue
        if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) { Register-KeepAlive }
        Write-Host "Other devices no longer get through Windows to Thoric. BIND_ADDRESS in .env is unchanged."
    }

    function Show-Lan {
        if (-not (Test-Registered)) {
            Write-Host "The $Distro distribution is not registered."
            return
        }
        $settings = Get-Settings
        $lanPort = Get-Port $settings
        $bind = $settings["BIND_ADDRESS"]
        if (-not $bind) { $bind = "127.0.0.1" }
        $mode = Get-NetworkingMode
        $rulePort = Get-RulePort
        $hyperVPort = Get-RulePort -HyperV
        Write-Host "Manager:        listens on ${bind}:$lanPort"
        Write-Host "WSL networking: $mode"
        if ($hyperVPort) { Write-Host "Hyper-V rule:   lets port $hyperVPort in" }
        if ($rulePort) { Write-Host "Firewall rule:  lets port $rulePort in" }
        $open = 0
        $problem = ""
        if (Test-Loopback $bind) {
            $problem = "the manager listens on this computer only; set BIND_ADDRESS=0.0.0.0 with 'thoric settings'"
        } elseif ($mode -eq "mirrored") {
            if ($hyperVPort) { $open = $hyperVPort } else { $problem = "the Hyper-V firewall keeps them out; run 'lan enable'" }
        } elseif ($mode -eq "nat") {
            $proxyPort = if ($rulePort) { $rulePort } else { $lanPort }
            $target = Get-PortProxy $proxyPort
            $address = Get-DistroAddress
            if ($target) { Write-Host "Port proxy:     port $proxyPort to ${target}:$proxyPort" }
            if (Get-ScheduledTask -TaskName $lanTaskName -ErrorAction SilentlyContinue) { Write-Host "LAN task:       '$lanTaskName' follows the address after WSL restarts" }
            if (-not $target) {
                $problem = "Windows does not forward the port; run 'lan enable'"
            } elseif (-not $rulePort) {
                $problem = "the Windows firewall has no rule for the port; run 'lan enable'"
            } elseif ($target -ne $address) {
                $problem = "the port proxy points at an old address, $Distro now has $address; run 'start'"
            } else {
                $open = $rulePort
            }
        } else {
            $problem = "'lan' handles NAT and mirrored networking only"
        }
        if ($open) {
            foreach ($ip in Get-WindowsAddresses) { Write-Host "Other devices:  http://${ip}:$open" }
        } else {
            Write-Host "Other devices:  cannot get in, $problem."
        }
    }

    # 'irm | iex' cannot pass a command, so without one the helper asks. Installing, it asks for the disk
    # once WSL is ready for the package.
    $askDisk = $false
    if (-not $Command) {
        if (-not (Test-Registered)) {
            Write-Host "Thoric is not installed yet."
            $Command = "install"
            $askDisk = $true
        } else {
            $state = if ((Get-Running) -contains $Distro) { "running" } else { "stopped" }
            $folder = Get-DiskFolder
            if ($folder) { Write-Host "Thoric is $state, with its disk in $folder." } else { Write-Host "Thoric is $state." }
            $autostart = [bool](Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)
            # Reading the port from .env would boot a stopped distribution, and with it the manager; the
            # firewall rules and the LAN task show a setup without it.
            $lan = Test-LanSetUp $(if ($state -eq "running") { Get-Port } elseif ($Port) { $Port } else { 8080 })
            $choices = @(
                @("start", "Start it and keep it running"),
                @("stop", "Stop it"),
                @("status", "Show the manager state"),
                @("open", "Open the dashboard"),
                @("update", "Update it to the newest release"),
                @("move", "Move its disk to another folder"),
                @("resize", "Change its disk's maximum size"),
                @("autostart", $(if ($autostart) { "Stop starting it at Windows sign-in" } else { "Start it at every Windows sign-in" })),
                @("lan", $(if ($lan) { "Stop other devices reaching the dashboard" } else { "Let other devices reach the dashboard" })),
                @("wslconfig", "Set WSL's memory, processors and swap"),
                @("uninstall", "Remove it with its images, containers and volumes")
            )
            for ($i = 0; $i -lt $choices.Count; $i++) { Write-Host ("  {0,2}  {1}" -f ($i + 1), $choices[$i][1]) }
            $answer = Read-Host "Number (Enter to leave)"
            if (-not $answer) { return }
            $number = 0
            if (-not [int]::TryParse($answer, [ref]$number) -or $number -lt 1 -or $number -gt $choices.Count) { throw "Choose a number from 1 to $($choices.Count)." }
            $Command = $choices[$number - 1][0]
            if ($Command -eq "move") { $Option = Read-Host "New folder for the disk, such as D:\WSL\Thoric" }
            if ($Command -eq "resize") { $Option = Read-Host "New maximum size, such as 200GB" }
            if ($Command -eq "autostart") { $Option = if ($autostart) { "disable" } else { "enable" } }
            if ($Command -eq "lan") { $Option = if ($lan) { "disable" } else { "enable" } }
        }
    }

    switch ($Command.ToLower()) {
        "install" {
            if (-not (Install-Wsl)) { return }
            if (Test-Registered) { throw "The $Distro distribution is already installed; 'update' moves it to the newest release." }
            Confirm-WslConfig
            if ($askDisk) {
                Write-Host "Thoric's disk will hold every image, container and volume."
                $Location = Read-Host "Folder for the disk, such as D:\WSL\Thoric (Enter for the WSL default)"
                $VhdSize = Read-Host "Maximum disk size, such as 200GB (Enter for 1 TB)"
            }
            $file = $Package
            if (-not $file) {
                $packageUrl = "$releaseUrl/$(Get-PackageName (Get-WindowsArchitecture))"
                $file = Join-Path $env:TEMP "thoric-$PID.wsl"
                Write-Host "Downloading $packageUrl"
                & curl.exe -fL --progress-bar -o $file $packageUrl
                if ($LASTEXITCODE -ne 0) { Remove-Item $file -ErrorAction SilentlyContinue; throw "The download failed: curl.exe exited with code $LASTEXITCODE." }
            } elseif (-not (Test-Path $file)) { throw "Package not found: $Package" }
            # The first start runs below, after the message; WSL would otherwise launch it as well.
            $arguments = @("--install", "--from-file", (Resolve-Path $file).Path, "--name", $Distro, "--no-launch")
            if ($Location) { $arguments += @("--location", (Resolve-Folder $Location)) }
            if ($VhdSize) { $arguments += @("--vhd-size", $VhdSize) }
            try { Invoke-Wsl $arguments } finally { if (-not $Package) { Remove-Item $file -ErrorAction SilentlyContinue } }
            Write-Host "The distribution is registered. Its first start asks for the username and password used by Linux and the dashboard."
            # The first start ends in a shell, whose last command gives the exit code.
            & wsl.exe -d $Distro
            if (-not (Test-Settings)) {
                Write-Host "The first start did not finish. Run 'wsl -d $Distro' to finish it, then the helper's 'start'."
                return
            }
            # WSL stops the distribution shortly after that shell ends, and Thoric with it.
            Write-Host "Starting Thoric and keeping it running."
            Start-Thoric
            if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
                if ((Read-Host "Start Thoric at every Windows sign-in too? Enter for yes, 'no' to skip") -in "", "y", "yes") {
                    Register-KeepAlive
                    Write-Host "Thoric will start when you sign in to Windows; 'autostart disable' turns that off."
                } else {
                    Write-Host "After Windows restarts, run the helper again and choose to start Thoric, or 'autostart enable' for every sign-in."
                }
            }
            Write-Host "Run the helper again to stop Thoric, open it to other devices or move its disk."
        }
        "start" {
            if (-not (Test-Registered)) { throw "The $Distro distribution is not registered. Run 'install' first." }
            Start-Thoric
        }
        "stop" {
            & wsl.exe -d $Distro -- thoric stop
            & wsl.exe --terminate $Distro
            Write-Host "Thoric stopped."
        }
        "status" {
            if (Test-Registered) { & wsl.exe -d $Distro -- thoric status } else { Write-Host "The $Distro distribution is not registered." }
        }
        "open" {
            Start-Process "http://localhost:$(Get-Port)"
        }
        "update" {
            if (-not (Test-Registered)) { throw "The $Distro distribution is not registered. Run 'install' first." }
            Invoke-Wsl @("-d", $Distro, "--", "thoric", "update")
        }
        "move" {
            if (-not $Option) { throw "Name the folder to move the disk to, for example 'move D:\WSL\Thoric'." }
            $folder = Resolve-Folder $Option
            Invoke-DiskChange @("--manage", $Distro, "--move", $folder) "Moving the disk to $folder. Copying a large disk to another drive takes a while."
        }
        "resize" {
            if (-not $Option) { throw "Give the disk's new maximum size, for example 'resize 200GB'." }
            Invoke-DiskChange @("--manage", $Distro, "--resize", $Option) "Setting the disk's maximum size to $Option."
        }
        "autostart" {
            switch ($Option.ToLower()) {
                "enable" {
                    Register-KeepAlive
                    Write-Host "Thoric will start and keep running after you sign in to Windows."
                }
                "disable" {
                    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
                    Write-Host "Auto-start removed."
                }
                default { throw "Use 'autostart enable' or 'autostart disable'." }
            }
        }
        "lan" {
            switch ($Option.ToLower()) {
                "enable" { Enable-Lan }
                "disable" { Disable-Lan }
                "status" { Show-Lan }
                default { throw "Use 'lan enable', 'lan disable' or 'lan status'." }
            }
        }
        "wslconfig" {
            Confirm-WslConfig
        }
        "elevated" {
            # The copy of this script that Invoke-Elevated starts as administrator. An error that leaves a
            # script run with -File ends powershell.exe with exit code 1.
            try {
                Invoke-AdministratorStep $Option $Port $Address $TaskUser
            } catch {
                if ($Log) { Set-Content -LiteralPath $Log -Value $_ }
                throw
            }
        }
        "uninstall" {
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
            if (Test-Registered) {
                Write-Host "This removes the $Distro distribution with its Docker images, desktops and volumes."
                $answer = Read-Host "Type 'remove' to continue"
                if ($answer -eq "remove") {
                    $lanPort = Get-Port
                    if (Test-LanSetUp $lanPort) {
                        Write-Host "Removing the way in for other devices too. Windows asks for administrator rights."
                        try {
                            Invoke-Elevated "none" $lanPort
                            Remove-Item -LiteralPath $addressFile -ErrorAction SilentlyContinue
                        } catch {
                            Write-Warning "$($_.Exception.Message) Run 'lan disable' later to remove the firewall rules and the port proxy."
                        }
                    }
                    Invoke-Wsl @("--unregister", $Distro)
                } else {
                    Write-Host "Kept the distribution."
                }
            }
        }
        default {
            Write-Host $usage
        }
    }
}

Invoke-Thoric @args
