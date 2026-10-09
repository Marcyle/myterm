# script/fetch-conpty.ps1
#
# 下载并解包现代 ConPTY（conpty.dll + OpenConsole.exe）到 Cargo 构建输出目录。
#
# 为什么需要：alacritty_terminal 的 Windows PTY 会优先从「exe 所在目录 / PATH」加载
# conpty.dll（见 alacritty_terminal::tty::windows::conpty::ConptyApi::new）。找不到时
# 退回系统内置 CreatePseudoConsole。内置 ConPTY 会把终端写回 PTY 输入侧的字节
# （OSC 10/11/4 颜色查询回复、DA 应答等）当作"键盘输入"重编，导致回包要么丢失、
# 要么以乱文本（如 ";rgb:0000/0000/0000"）漏到屏幕上——pi、vim、fzf 等依赖终端查询
# 的程序会因此异常。捆绑现代 ConPTY 后，这些字节原样直达子进程 stdin。
#
# 用法:
#   powershell -File script/fetch-conpty.ps1                 # 解包到 target\debug 和 target\release
#   powershell -File script/fetch-conpty.ps1 -TargetTriple x86_64-pc-windows-msvc
#   powershell -File script/fetch-conpty.ps1 -OutDir x       # 解包到指定目录
param(
    [string]$Version = "1.25.260930003",
    [string]$Arch = "x64",
    [string]$TargetTriple = "",
    [string]$OutDir = "",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

$packageName = "Microsoft.Windows.Console.ConPTY"
$nupkgUrl = "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$Version/microsoft.windows.console.conpty.$Version.nupkg"

# 与 www.nuget.org 上该版本包一致（重复下载字节级稳定）
$expectedSha256 = switch ($Arch) {
    "x64" { "02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee" }
    default { "" }  # 其他架构首次下载后按需补充
}

$repoRoot = Split-Path -Parent $PSScriptRoot

$dllMember = switch ($Arch) {
    "x64"   { "runtimes/win-x64/native/conpty.dll" }
    "x86"   { "runtimes/win-x86/native/conpty.dll" }
    "arm64" { "runtimes/win-arm64/native/conpty.dll" }
    default { throw "不支持的架构: $Arch" }
}
$exeMember = switch ($Arch) {
    "x64"   { "build/native/runtimes/x64/OpenConsole.exe" }
    "x86"   { "build/native/runtimes/x86/OpenConsole.exe" }
    "arm64" { "build/native/runtimes/arm64/OpenConsole.exe" }
}

$cacheDir = Join-Path $repoRoot "target/conpty-cache/$Version"
$nupkgPath = Join-Path $cacheDir "$packageName.$Version.nupkg"

New-Item -ItemType Directory -Force $cacheDir | Out-Null

if (-not (Test-Path $nupkgPath) -or $Force) {
    Write-Host "下载 $nupkgUrl"
    Invoke-WebRequest -Uri $nupkgUrl -OutFile $nupkgPath -UseBasicParsing
}

if ($expectedSha256) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $actual = (($sha.ComputeHash([IO.File]::ReadAllBytes($nupkgPath)) | ForEach-Object { $_.ToString('x2') }) -join '')
    if ($actual -ne $expectedSha256) {
        throw "校验失败: 期望 $expectedSha256 实际 $actual (可加 -Force 重新下载)"
    }
}

$targets = if ($OutDir) { @($OutDir) } else {
    $suffix = if ($TargetTriple) { "$TargetTriple/" } else { "" }
    @((Join-Path $repoRoot "target/${suffix}debug"), (Join-Path $repoRoot "target/${suffix}release"))
}

foreach ($dir in $targets) {
    New-Item -ItemType Directory -Force $dir | Out-Null
    & tar.exe -xf $nupkgPath -C $cacheDir $dllMember $exeMember
    if ($LASTEXITCODE -ne 0) { throw "tar 解包失败 ($LASTEXITCODE)" }
    Copy-Item (Join-Path $cacheDir $dllMember) (Join-Path $dir "conpty.dll") -Force
    Copy-Item (Join-Path $cacheDir $exeMember) (Join-Path $dir "OpenConsole.exe") -Force
    Write-Host "已安装 conpty.dll + OpenConsole.exe -> $dir"
}
