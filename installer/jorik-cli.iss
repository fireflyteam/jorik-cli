#define MyAppName "jorik-cli"
#ifndef MyAppVersion
  #if GetEnv("VERSION") != ""
    #define MyAppVersion GetEnv("VERSION")
  #else
    #define MyAppVersion "0.1.0"
  #endif
#endif
#ifndef MyTarget
  #if GetEnv("TARGET") != ""
    #define MyTarget GetEnv("TARGET")
  #else
    #define MyTarget "x86_64-pc-windows-msvc"
  #endif
#endif
#define MyExe "jorik.exe"

[Setup]
APPID={{BD6BB68C-5547-4FC4-A93D-7C322F0A1443}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppName}
DefaultDirName={userappdata}\{#MyAppName}
OutputDir=..\\dist
OutputBaseFilename={#MyAppName}-{#MyAppVersion}-{#MyTarget}-setup
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
Compression=lzma
SolidCompression=yes
ChangesEnvironment=yes
PrivilegesRequired=lowest
UsePreviousAppDir=yes
DisableDirPage=yes
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyExe}

[Files]
Source: "..\target\{#MyTarget}\release\{#MyExe}"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Flags: preservestringtype uninsdeletevalue

[Run]
Filename: "{app}\{#MyExe}"; Description: "Run {#MyAppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
