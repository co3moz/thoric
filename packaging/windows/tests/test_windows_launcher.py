"""The Windows launcher, packaging/windows/thoric.ps1, which releases attach next to the package
and nothing else runs before a user does: that Windows PowerShell 5.1 reads it as written, that its
auto-start brings the manager back, that it parses, that its administrator side, the copy of itself that 'lan' starts through UAC, runs
nothing from the distribution, whose user has passwordless sudo, that the settings it offers
for .wslconfig fit the computer and leave the rest of the file alone, and that it downloads the
package of Windows' architecture under the name the release attaches it with.

All but the first need PowerShell: pwsh on Linux, or Windows PowerShell from Windows or WSL.
"""

import fnmatch
import json
from pathlib import Path
import re
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "packaging/windows/thoric.ps1"
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell") or shutil.which("powershell.exe")

# Reads the script from stdin and walks from the 'elevated' command through every function of the script
# it calls, collecting their commands and strings. One line without double quotes, so it survives any
# command line.
ANALYZE = " ".join("""
$t = [Console]::In.ReadToEnd(); $e = $null;
$a = [System.Management.Automation.Language.Parser]::ParseInput($t, [ref]$null, [ref]$e);
$f = @{}; foreach ($d in $a.FindAll({ $args[0] -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)) { $f[$d.Name] = $d };
$todo = New-Object System.Collections.Queue;
foreach ($s in $a.FindAll({ $args[0] -is [System.Management.Automation.Language.SwitchStatementAst] }, $true)) {
  foreach ($c in $s.Clauses) { if ($c.Item1.Value -eq 'elevated') { $todo.Enqueue($c.Item2) } } };
$seen = @{}; $commands = @(); $strings = @();
while ($todo.Count) { $n = $todo.Dequeue();
  foreach ($c in $n.FindAll({ $args[0] -is [System.Management.Automation.Language.CommandAst] }, $true)) {
    $name = $c.GetCommandName();
    if ($name) { $commands += $name; if ($f.ContainsKey($name) -and -not $seen[$name]) { $seen[$name] = 1; $todo.Enqueue($f[$name].Body) } } };
  foreach ($s in $n.FindAll({ $args[0] -is [System.Management.Automation.Language.StringConstantExpressionAst] -or
      $args[0] -is [System.Management.Automation.Language.ExpandableStringExpressionAst] }, $true)) { $strings += $s.Value } };
@{ errors = @($e | ForEach-Object { $_.Message }); commands = $commands; strings = $strings; functions = @($seen.Keys) } | ConvertTo-Json -Compress
""".split())

# Reads the script, the names of some of its functions and PowerShell expressions as JSON from stdin,
# defines those functions on their own and prints what each expression returns. Like ANALYZE, one line
# without double quotes.
CALL = " ".join("""
$i = [Console]::In.ReadToEnd() | ConvertFrom-Json;
$a = [System.Management.Automation.Language.Parser]::ParseInput($i.script, [ref]$null, [ref]$null);
foreach ($d in $a.FindAll({ $args[0] -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
  if ($i.functions -contains $d.Name) { . ([scriptblock]::Create($d.Extent.Text)) } };
ConvertTo-Json -Compress -Depth 5 -InputObject @($i.calls | ForEach-Object { , (& ([scriptblock]::Create($_))) })
""".split())


def call(*expressions, functions=("Get-WslSuggestion", "Read-WslConfig", "Add-WslSettings")):
    """What each PowerShell expression returns with the script's own functions defined."""
    request = json.dumps({"script": SCRIPT.read_text(), "functions": list(functions), "calls": list(expressions)})
    result = subprocess.run([POWERSHELL, "-NoProfile", "-NonInteractive", "-Command", CALL],
                            input=request, capture_output=True, text=True, timeout=120)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return json.loads(result.stdout)


class WindowsLauncherTest(unittest.TestCase):
    def test_auto_start_brings_a_stopped_manager_back(self):
        # 'stop' runs 'thoric stop', after which Compose's 'restart: unless-stopped' leaves the manager
        # down, so the task runs 'thoric start' before it keeps the distribution alive with the package's
        # keep-alive process.
        script = SCRIPT.read_text()
        command = re.search(r'(?m)^    \$autoStart = "([^"]+)"$', script).group(1)
        self.assertRegex(command, r"^thoric start\b[^;]*; exec (\S+)$")
        keepalive = command.rsplit(" ", 1)[1]
        self.assertRegex((ROOT / "packaging/wsl/Dockerfile").read_text(),
                         rf"(?m)^COPY --chmod=0755 packaging/wsl/keepalive\.sh {re.escape(keepalive)}$")
        self.assertIn('-Argument "-d $Distro -- sh -c `"$autoStart`""', script)
        # 'start' finds the keep-alive of this task, and of the tasks earlier helpers registered, by its
        # command line, which is matched as PowerShell's -like does, ignoring case.
        pattern = re.search(r'\$_\.CommandLine -like "([^"]+)"', script).group(1).replace("$Distro", "Thoric")
        for line in (f'"wsl.exe" -d Thoric -- sh -c "{command}"', f"wsl.exe -d Thoric -- {keepalive}"):
            self.assertTrue(fnmatch.fnmatchcase(line.lower(), pattern.lower()), line)
        self.assertFalse(fnmatch.fnmatchcase(f"wsl.exe -d Thoric2 -- {keepalive}".lower(), pattern.lower()))

    def test_windows_powershell_reads_it_as_written(self):
        # Windows PowerShell 5.1 reads a script without a byte order mark in the ANSI code page, so any
        # character outside ASCII would reach users garbled.
        SCRIPT.read_bytes().decode("ascii")

    @unittest.skipUnless(POWERSHELL, "needs PowerShell")
    def test_the_administrator_side_runs_nothing_from_the_distribution(self):
        result = subprocess.run([POWERSHELL, "-NoProfile", "-NonInteractive", "-Command", ANALYZE],
                                input=SCRIPT.read_text(), capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["errors"], [])
        commands = [command.lower() for command in report["commands"]]
        # The walk reached the firewall and port proxy changes, so it followed the step's functions.
        self.assertIn("netsh.exe", commands)
        self.assertIn("new-netfirewallhypervrule", commands)
        self.assertEqual([command for command in commands if re.fullmatch(r"wsl(\.exe)?", command)], [])
        # The LAN task's command is a string, so no string may start wsl either.
        self.assertEqual([text for text in report["strings"] if re.search(r"(?i)\bwsl(\.exe)?(\s|$)", text)], [])

    @unittest.skipUnless(POWERSHELL, "needs PowerShell")
    def test_wsl_settings_leave_windows_its_share(self):
        big, small, tiny = call("Get-WslSuggestion 128 24", "Get-WslSuggestion 16 8", "Get-WslSuggestion 4 2")
        # A quarter of the memory and processors stays with Windows, and swap stops at 8 GB.
        self.assertEqual(big, {"memory": "96GB", "processors": "18", "swap": "8GB"})
        self.assertEqual(small, {"memory": "12GB", "processors": "6", "swap": "3GB"})
        # WSL still gets 2 GB on a small computer, and four processors or fewer are all shared.
        self.assertEqual(tiny, {"memory": "2GB", "processors": "2", "swap": "1GB"})

    @unittest.skipUnless(POWERSHELL, "needs PowerShell")
    def test_wsl_settings_are_added_without_touching_the_rest(self):
        lines = ["[boot]", "systemd=true", "", "[WSL2]", "networkingMode = mirrored", "# swap=0", "",
                 "[experimental]", "memory=1GB"]
        given = "@(" + ", ".join(f"'{line}'" for line in lines) + ")"
        settings = "([ordered]@{ memory = '96GB'; swap = '8GB' })"
        read, inserted, appended, created = call(
            f"Read-WslConfig {given}",
            f"Add-WslSettings {given} {settings}",
            f"Add-WslSettings @('[boot]', 'systemd=true', '', '') {settings}",
            f"Add-WslSettings @() {settings}",
        )
        # Only [wsl2] counts, whatever its case, and a comment sets nothing.
        self.assertEqual(read, {"networkingmode": "mirrored"})
        self.assertEqual(inserted, lines[:4] + ["memory=96GB", "swap=8GB"] + lines[4:])
        self.assertEqual(appended, ["[boot]", "systemd=true", "", "[wsl2]", "memory=96GB", "swap=8GB"])
        self.assertEqual(created, ["[wsl2]", "memory=96GB", "swap=8GB"])

    def test_the_release_attaches_the_package_of_each_platform(self):
        # build.sh names a package after its platform, the workflow builds each platform's under that
        # name and attaches every one beside the launcher.
        self.assertEqual(built_packages(), {"amd64": "thoric.wsl", "arm64": "thoric-arm64.wsl"})
        workflow = (ROOT / ".github/workflows/publish.yml").read_text()
        matrix = re.findall(r"(?m)^ +- arch: (\S+)\n +runner: \S+\n +package: (\S+)$", workflow)
        self.assertEqual(dict(matrix), built_packages())
        attached = re.search(r"(?m)^ +gh release upload \S+ (.+) --clobber$", workflow).group(1).split()
        self.assertEqual(attached, [f"dist/{name}" for name in built_packages().values()] + ["packaging/windows/thoric.ps1"])

    @unittest.skipUnless(POWERSHELL, "needs PowerShell")
    def test_install_downloads_the_package_of_windows_architecture(self):
        x64, arm, lower, other = call(
            "Get-PackageName AMD64", "Get-PackageName ARM64", "Get-PackageName arm64",
            "try { Get-PackageName x86 } catch { $_.Exception.Message }",
            functions=("Get-PackageName",),
        )
        self.assertEqual([x64, arm], [built_packages()["amd64"], built_packages()["arm64"]])
        # PROCESSOR_ARCHITECTURE is upper case, but a name is not refused for its case.
        self.assertEqual(lower, arm)
        self.assertEqual(other, "WSL 2 runs on x64 and ARM64 Windows, not on x86.")
        # The download takes the name of the machine's architecture, never a fixed one.
        script = SCRIPT.read_text()
        self.assertIn('$packageUrl = "$releaseUrl/$(Get-PackageName (Get-WindowsArchitecture))"', script)
        self.assertNotRegex(script, r"download/thoric(-\w+)?\.wsl")


def built_packages():
    """The package build.sh writes for each platform when no output is named, by architecture."""
    return dict(re.findall(r"(?m)^    linux/(\w+)\) package=(\S+) ;;$", (ROOT / "packaging/wsl/build.sh").read_text()))


if __name__ == "__main__":
    unittest.main()
