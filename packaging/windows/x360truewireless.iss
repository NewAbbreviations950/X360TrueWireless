; X360TrueWireless for Windows. Built by the release workflow:
;   iscc /DAppVersion=0.2.0 /DStage=<folder with the exes and prerequisites> x360truewireless.iss
;
; Installs the driver and tray app, ViGEmBus (virtual Xbox pads) and vJoy (virtual wheels) if
; they're missing, sets up vJoy devices 1-4 as wheels, puts WinUSB on the receiver, and starts
; both programs at every sign-in.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Stage
  #define Stage "stage"
#endif
#define ViGEmSetup "ViGEmBus_1.22.0_x64_x86_arm64.exe"
#define VJoySetup "vJoySetup_v2.2.2.0_Win10_Win11.exe"

[Setup]
AppId={{6E3C1A52-8B0D-4F6B-9C2E-5A1D7F3B9E40}
AppName=X360TrueWireless
AppVersion={#AppVersion}
AppPublisher=X360TrueWireless
AppPublisherURL=https://github.com/NewAbbreviations950/X360TrueWireless
DefaultDirName={autopf}\X360TrueWireless
DefaultGroupName=X360TrueWireless
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=.
OutputBaseFilename=X360TrueWireless-Setup-x86_64
UninstallDisplayIcon={app}\x360truewireless-tray.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=force

[Files]
Source: "{#Stage}\x360truewireless-daemon.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\x360truewireless-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "receiver-driver.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\{#ViGEmSetup}"; DestDir: "{tmp}"; Flags: deleteafterinstall
Source: "{#Stage}\{#VJoySetup}"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{group}\X360TrueWireless Settings"; Filename: "{app}\x360truewireless-tray.exe"; Parameters: "--settings"
Name: "{group}\Set up receiver"; Filename: "{app}\x360truewireless-tray.exe"; Parameters: "--setup-receiver"

[Registry]
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "X360TrueWireless"; ValueData: """{app}\x360truewireless-daemon.exe"""; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "X360TrueWireless Tray"; ValueData: """{app}\x360truewireless-tray.exe"""; Flags: uninsdeletevalue

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/IM x360truewireless-tray.exe /F"; Flags: runhidden; RunOnceId: "StopTray"
Filename: "{sys}\taskkill.exe"; Parameters: "/IM x360truewireless-daemon.exe /F"; Flags: runhidden; RunOnceId: "StopDriver"
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\receiver-driver.ps1"" -Uninstall"; Flags: runhidden; RunOnceId: "RemoveDriver"

[Code]
function ServiceInstalled(Name: String): Boolean;
begin
  Result := RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\' + Name);
end;

procedure RunOrWarn(FileName, Params, What: String);
var
  Code: Integer;
begin
  if not Exec(FileName, Params, '', SW_HIDE, ewWaitUntilTerminated, Code) or (Code <> 0) then
    MsgBox(What + ' failed (code ' + IntToStr(Code) + ').', mbError, MB_OK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code, Device: Integer;
  VJoyConfig: String;
begin
  if CurStep <> ssPostInstall then
    exit;

  if not ServiceInstalled('ViGEmBus') then begin
    WizardForm.StatusLabel.Caption := 'Installing ViGEmBus (virtual Xbox controllers)...';
    RunOrWarn(ExpandConstant('{tmp}\{#ViGEmSetup}'), '/exenoui /qn /norestart', 'Installing ViGEmBus');
  end;

  if not ServiceInstalled('vjoy') then begin
    WizardForm.StatusLabel.Caption := 'Installing vJoy (virtual racing wheels)...';
    RunOrWarn(ExpandConstant('{tmp}\{#VJoySetup}'), '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', 'Installing vJoy');
  end;

  // One wheel per controller slot: steering, throttle, brake, 11 buttons, a hat, force feedback.
  VJoyConfig := ExpandConstant('{commonpf64}\vJoy\x64\vJoyConfig.exe');
  if FileExists(VJoyConfig) then begin
    WizardForm.StatusLabel.Caption := 'Setting up vJoy wheels...';
    for Device := 1 to 4 do
      RunOrWarn(VJoyConfig, IntToStr(Device) + ' -f -a x y rz -b 11 -p 1 -e All', 'Setting up vJoy device ' + IntToStr(Device));
  end;

  WizardForm.StatusLabel.Caption := 'Setting up the receiver...';
  Exec('powershell.exe', '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\receiver-driver.ps1') + '"',
    '', SW_HIDE, ewWaitUntilTerminated, Code);
  if Code = 2 then
    MsgBox('The receiver isn''t plugged in. Plug it in, then open "Set up receiver" from the Start menu.', mbInformation, MB_OK)
  else if Code <> 0 then
    MsgBox('Setting up the receiver failed (code ' + IntToStr(Code) + '). Try "Set up receiver" from the Start menu.', mbError, MB_OK);

  // Start both for the signed-in user (they also start at every sign-in).
  ExecAsOriginalUser(ExpandConstant('{app}\x360truewireless-daemon.exe'), '', '', SW_SHOWNORMAL, ewNoWait, Code);
  ExecAsOriginalUser(ExpandConstant('{app}\x360truewireless-tray.exe'), '', '', SW_SHOWNORMAL, ewNoWait, Code);
end;
