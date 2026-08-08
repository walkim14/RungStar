; Windows installer, for Inno Setup 6.
;
;   iscc packaging\windows\rungstar.iss
;
; Run packaging\windows\portable.ps1 first: this packages what that assembled, rather than
; listing the DLLs again in a second place that can disagree with the first.

#define Name "RungStar"
#define Version "0.1.0"
#define Publisher "RungStar contributors"
#define Url "https://github.com/walkim/rungstar"
#define Exe "rungstar.exe"
#define Staged "..\..\target\package\RungStar"

[Setup]
AppId={{6F1C4B1E-9E1B-4C1D-9E4B-7A2D5C3E9A11}
AppName={#Name}
AppVersion={#Version}
AppPublisher={#Publisher}
AppPublisherURL={#Url}
DefaultDirName={autopf}\{#Name}
DefaultGroupName={#Name}
; The licence is shown rather than buried: this is GPL-3.0-or-later, and somebody installing it
; is entitled to know what they may do with it.
LicenseFile=..\..\LICENSE
OutputDir=..\..\target\package
OutputBaseFilename=RungStar-{#Version}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; 64-bit only. The vendored FFmpeg and SDL3 are x64 and there is no 32-bit build to offer.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; No admin rights needed when installing per-user, which is what most people want and what
; works on a machine somebody does not administer.
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#Staged}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#Name}"; Filename: "{app}\{#Exe}"
Name: "{group}\Microphone check"; Filename: "{app}\rungstar-diagnostics.exe"; Comment: "Test microphones, pitch detection and controllers"
Name: "{group}\Uninstall {#Name}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#Name}"; Filename: "{app}\{#Exe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#Exe}"; Description: "Start {#Name}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Only what the installer put there. Settings, songs, profiles and the USDB catalog live in
; {userappdata}\RungStar and are deliberately left alone: an uninstall should not be able to
; delete somebody's highscores, and a reinstall should find them again.
Type: filesandordirs; Name: "{app}\assets"
