# 在一般 PowerShell 執行；登入後啟動，讓瀏覽器與通知留在桌面工作階段。
param([string]$Executable = "$env:USERPROFILE\.cargo\bin\trserial.exe")
$Action = New-ScheduledTaskAction -Execute $Executable -Argument 'daemon'
$Trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$Principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$Settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName 'TRserial' -Action $Action -Trigger $Trigger -Principal $Principal -Settings $Settings -Description 'Process the local TRserial queue after desktop login'
