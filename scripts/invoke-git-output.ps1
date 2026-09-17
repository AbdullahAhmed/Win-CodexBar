#Requires -Version 5.1

function Invoke-WinCodexBarGit {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Repository,
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $savedErrorActionPreference = $ErrorActionPreference
    try {
        # PowerShell 5.1 can promote native stderr to an error record. Git's
        # warnings are non-fatal here; the exit code remains authoritative.
        $ErrorActionPreference = 'Continue'
        $output = @(& git -C $Repository @Arguments 2>$null)
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $savedErrorActionPreference
    }

    return [pscustomobject]@{
        Output = $output
        ExitCode = $exitCode
    }
}
