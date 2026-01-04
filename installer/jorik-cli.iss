#define MyAppName "jorik-cli"
#ifndef MyAppVersion
  #if GetEnv("VERSION") != ""
    #define MyAppVersion GetEnv("VERSION")
  #else
    #define MyAppVersion "0.2.0"
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

[Code]
const
  VC_REDIST_URL = 'https://aka.ms/vs/17/release/vc_redist.x64.exe';

function VCRedistNeedsInstall: Boolean;
var
  Installed: Cardinal;
begin
  // Check for Visual C++ 2015-2022 Redistributable (x64)
  // Registry key: HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64
  if RegQueryDWordValue(HKEY_LOCAL_MACHINE, 'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64', 'Installed', Installed) and (Installed = 1) then
  begin
    Result := False;
  end
  else
  begin
    Result := True;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  DownloadPath: String;
begin
  if (CurStep = ssPostInstall) and VCRedistNeedsInstall then
  begin
    if MsgBox('This application requires the Microsoft Visual C++ Redistributable (x64). Download and install it now?', mbConfirmation, MB_YESNO) = IDYES then
    begin
      DownloadPath := ExpandConstant('{tmp}\vc_redist.x64.exe');
      try
        DownloadTemporaryFile(VC_REDIST_URL, 'vc_redist.x64.exe', '', nil);
        // Run the installer. It will prompt for UAC if necessary.
        Exec(DownloadPath, '/install /passive /norestart', '', SW_SHOW, ewWaitUntilTerminated, ResultCode);
      except
        MsgBox('Error downloading or installing Visual C++ Redistributable: ' + GetExceptionMessage, mbError, MB_OK);
      end;
    end;
  end;
end;
