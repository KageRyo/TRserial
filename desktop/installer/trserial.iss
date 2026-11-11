; Per-user Windows installer, built by the draft release workflow:
;   iscc /DAppVersion=0.1.0 /DAppBinaryVersion=0.1.0 /DSourceDir=<staging> [/DIconFile=<ico>] /O<dir> trserial.iss
; SourceDir holds trserial-desktop.exe, trserial.exe, README.md, LICENSE.txt
; and config.example.toml.
#ifndef AppVersion
  #error Define AppVersion, for example /DAppVersion=0.1.0
#endif
#ifndef AppBinaryVersion
  #define AppBinaryVersion AppVersion
#endif
#ifndef SourceDir
  #error Define SourceDir as the staged Windows release folder
#endif
#ifndef OutputName
  #define OutputName "trserial-windows-amd64-setup"
#endif

[Setup]
; Keep this AppId: Windows uses it to upgrade and uninstall the same app.
AppId={{F6DF1C0A-902B-4C75-9DEC-81148DA92772}
AppName=TRserial
AppVersion={#AppVersion}
AppVerName=TRserial {#AppVersion}
AppPublisher=KageRyo
AppPublisherURL=https://github.com/KageRyo/TRserial
AppSupportURL=https://github.com/KageRyo/TRserial/issues
AppUpdatesURL=https://github.com/KageRyo/TRserial/releases/latest
LicenseFile="{#SourceDir}\LICENSE.txt"
#ifdef IconFile
SetupIconFile="{#IconFile}"
#endif
; Per-user install: no administrator prompt.
DefaultDirName={localappdata}\Programs\TRserial
DefaultGroupName=TRserial
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
UninstallDisplayName=TRserial
UninstallDisplayIcon={app}\trserial-desktop.exe
OutputBaseFilename={#OutputName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
ChangesAssociations=no
VersionInfoCompany=KageRyo
VersionInfoDescription=TRserial Windows installer
VersionInfoProductName=TRserial
VersionInfoVersion={#AppBinaryVersion}
VersionInfoProductVersion={#AppBinaryVersion}
VersionInfoTextVersion={#AppVersion}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesetraditional"; MessagesFile: "languages\ChineseTraditional.isl"

[LangOptions]
chinesetraditional.LanguageName=正體中文

[CustomMessages]
english.CreateDesktopShortcut=Create a desktop shortcut
chinesetraditional.CreateDesktopShortcut=建立桌面捷徑
english.AdditionalIcons=Additional icons:
chinesetraditional.AdditionalIcons=其他圖示：
english.LaunchTRserial=Launch TRserial
chinesetraditional.LaunchTRserial=啟動 TRserial

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopShortcut}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\trserial-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\trserial.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: isreadme
Source: "{#SourceDir}\LICENSE.txt"; DestDir: "{app}"; DestName: "LICENSE"; Flags: ignoreversion
Source: "{#SourceDir}\config.example.toml"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\TRserial"; Filename: "{app}\trserial-desktop.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\TRserial"; Filename: "{app}\trserial-desktop.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
; The app writes this value when "start at login" is on. Remove it with the
; program so Windows does not try to start a deleted file. Account data and
; redemption history in the user profile are kept.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "TRserial"; ValueType: none; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\trserial-desktop.exe"; Description: "{cm:LaunchTRserial}"; Flags: nowait postinstall skipifsilent
