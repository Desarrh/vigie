# Mesure de Vigie au repos, sans deranger l'utilisateur (regle 0) :
# fenetre de test invisible (--test-hors-ecran du socle : opacite 0, sans focus, priorite basse),
# donnees dans target\test-data-mesure (VIGIE_DATA_DIR), 15 s de chauffe puis 60 s au repos.
# Usage : powershell -File tools\mesure.ps1 [-reduite]   (-reduite : fenetre reduite sans activation)
param([switch]$reduite, [string]$exe = "$PSScriptRoot\..\target\release\vigie.exe")
$data = Join-Path $PSScriptRoot "..\target\test-data-mesure"
New-Item -ItemType Directory -Force $data | Out-Null
$env:VIGIE_DATA_DIR = (Resolve-Path $data).Path
if ($reduite) { $env:VIGIE_TEST_REDUIT = "1" } else { Remove-Item Env:VIGIE_TEST_REDUIT -ErrorAction SilentlyContinue }
Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();' -Name W -Namespace VigieMesure -ErrorAction SilentlyContinue
$fg0 = [VigieMesure.W]::GetForegroundWindow()
$p = Start-Process $exe -ArgumentList "--test-hors-ecran" -PassThru
Start-Sleep 15
$p.Refresh(); $c1 = $p.TotalProcessorTime
Start-Sleep 60
$p.Refresh(); $c2 = $p.TotalProcessorTime
$fg1 = [VigieMesure.W]::GetForegroundWindow()
$nb = (Get-Process).Count
$charge = (Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average
"{0} : prive {1:N1} Mo, plage de travail {2:N1} Mo, CPU {3:N0} ms sur 60 s ({4:N2} % d'un coeur), {5} processus, charge machine {6} %, focus inchange : {7}" -f $(if ($reduite) { "REDUITE" } else { "OUVERTE" }), ($p.PrivateMemorySize64/1MB), ($p.WorkingSet64/1MB), ($c2-$c1).TotalMilliseconds, (($c2-$c1).TotalMilliseconds/600), $nb, $charge, ($fg0 -eq $fg1)
Stop-Process -Id $p.Id -Force
Remove-Item Env:VIGIE_DATA_DIR, Env:VIGIE_TEST_REDUIT -ErrorAction SilentlyContinue
Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
