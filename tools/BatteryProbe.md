# Read-only Windows battery probe

`BatteryProbe.cs` uses Windows battery-class QUERY requests only. It writes a new
JSON report and refuses to overwrite a file. No driver installation or battery
configuration changes are needed. Serial numbers and device paths are omitted.
Run it on demand, outside battery-efficiency measurements.

Build from the repository root in PowerShell with the installed .NET Framework:

```powershell
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /platform:x64 /r:System.Web.Extensions.dll /out:diagnostic-reports\research\BatteryProbe.exe tools\BatteryProbe.cs
& .\diagnostic-reports\research\BatteryProbe.exe --output diagnostic-reports\research\battery-new.json
```

The output directory must exist. Unknown/unsupported fields remain null with an
error. Raw capacity and rate units depend on `capacity_is_relative`; absolute
capacities use mWh and rates mW. Signed negative rates indicate discharge; zero
while plugged in is not application power consumption. This Blade 18 exposes
capacity/status, but its battery temperature query returns unsupported (error 1).

API reference: [Microsoft battery query documentation](https://learn.microsoft.com/windows/win32/power/ioctl-battery-query-information).
