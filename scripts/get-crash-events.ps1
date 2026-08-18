$events = Get-WinEvent -FilterHashtable @{LogName='Application'; Id=1000} -MaxEvents 120 |
    Where-Object { $_.Message -like '*OpenCADStudio*' } |
    Select-Object -First 3
foreach ($event in $events) {
    Write-Output ("TIME: " + $event.TimeCreated)
    $text = $event.Message
    if ($text.Length -gt 1100) { $text = $text.Substring(0, 1100) }
    Write-Output $text
    Write-Output "-----"
}
if (-not $events) { Write-Output "no OpenCADStudio crash events found in Id=1000" }
