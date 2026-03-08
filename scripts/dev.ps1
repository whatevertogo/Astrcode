$ErrorActionPreference = 'Stop'

$frontendPort = 5173
$frontendUrl = "http://localhost:$frontendPort"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$frontendDir = Join-Path $repoRoot 'frontend'
$frontendProcess = $null

function Get-PortProcessIds {
    param([int]$Port)

    $connections = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue
    if ($null -eq $connections) {
        return @()
    }

    return @($connections | Select-Object -ExpandProperty OwningProcess -Unique)
}

function Stop-Frontend {
    if ($null -ne $frontendProcess -and -not $frontendProcess.HasExited) {
        Write-Host ""
        Write-Host "[stop] 终止前端进程..." -ForegroundColor Yellow
        Stop-Process -Id $frontendProcess.Id -Force -ErrorAction SilentlyContinue
        $frontendProcess.WaitForExit()
    }
}

function Test-PortInUse {
    param([int]$Port)

    return (Get-PortProcessIds -Port $Port).Count -gt 0
}

function Test-FrontendReady {
    param([string]$Url)

    try {
        $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
        return $response.StatusCode -ge 200 -and $response.StatusCode -lt 500
    } catch {
        return $false
    }
}

function Wait-FrontendReady {
    param(
        [string]$Url,
        [int]$TimeoutSeconds
    )

    for ($attempt = 1; $attempt -le $TimeoutSeconds; $attempt++) {
        if (Test-FrontendReady -Url $Url) {
            Write-Host "✓ 前端已就绪: $Url" -ForegroundColor Green
            return
        }

        Write-Host "[wait] 等待前端启动... ($attempt/$TimeoutSeconds)"
        Start-Sleep -Seconds 1
    }

    throw "前端在 $TimeoutSeconds 秒内未就绪：$Url"
}

trap {
    Stop-Frontend
    throw
}

try {
    Write-Host "[check] 检查端口 $frontendPort ..." -ForegroundColor Cyan
    if (Test-PortInUse -Port $frontendPort) {
        Write-Warning "端口 $frontendPort 已被占用。"
        if (Test-FrontendReady -Url $frontendUrl) {
            $answer = Read-Host "检测到现有前端可访问。继续复用？输入 y 继续，其他任意键退出"
            if ($answer -ne 'y' -and $answer -ne 'Y') {
                throw "用户取消启动。"
            }
            Write-Host "[info] 复用现有前端服务。"
        } else {
            Write-Warning "端口已占用，但当前前端不可访问，可能是残留/异常进程。"
            $answer = Read-Host "是否终止占用进程并重新启动前端？输入 y 继续，其他任意键退出"
            if ($answer -ne 'y' -and $answer -ne 'Y') {
                throw "用户取消启动。"
            }

            foreach ($owningProcess in Get-PortProcessIds -Port $frontendPort) {
                Write-Host "[stop] 终止占用端口的进程 PID=$owningProcess" -ForegroundColor Yellow
                Stop-Process -Id $owningProcess -Force -ErrorAction SilentlyContinue
            }

            Start-Sleep -Seconds 1
            if (Test-PortInUse -Port $frontendPort) {
                throw "端口 $frontendPort 仍被占用，无法启动前端。"
            }
        }
    }

    if (-not (Test-PortInUse -Port $frontendPort)) {
        Write-Host "[start] 在新窗口启动前端开发服务器..." -ForegroundColor Cyan
        $frontendCommand = @"
Set-Location '$frontendDir'
`$Host.UI.RawUI.WindowTitle = 'AstrCode Frontend'
npm run dev
"@
        $frontendProcess = Start-Process powershell `
            -ArgumentList '-NoExit', '-ExecutionPolicy', 'Bypass', '-Command', $frontendCommand `
            -PassThru
    }

    Wait-FrontendReady -Url $frontendUrl -TimeoutSeconds 60

    Write-Host "[start] 启动 Tauri 开发环境..." -ForegroundColor Cyan
    Set-Location $repoRoot
    cargo tauri dev
} finally {
    Stop-Frontend
}
