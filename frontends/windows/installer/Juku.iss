; The Windows installer (spec section 11.8), made with Inno Setup 6
; (https://jrsoftware.org/isinfo.php). build.cmd dist runs it:
;
;   ISCC.exe /DSource=<folder with Juku.exe and JukuThumbnail.dll> /O<output folder> Juku.iss
;
; It installs for the current user only, into %LOCALAPPDATA%\Programs\Juku,
; so it needs no administrator rights, like Settings -> Register File Types,
; which writes to the user's part of the registry too. The file types are
; registered by Juku.exe itself (--register, and --unregister on uninstall),
; so the installer and the Settings button write the same keys.
;
; The version, the publisher and the copyright are read from Juku.exe's
; version information, which build.rs takes from VERSION and the Mac app's
; Info.plist.
; The settings and files in %LOCALAPPDATA%\Juku stay when Juku is
; uninstalled, as the Mac's do when the app is deleted.

#ifndef Source
  #define Source "..\..\..\target\windows"
#endif

#define Exe Source + "\Juku.exe"
#define Version GetStringFileInfo(Exe, "ProductVersion")
#define Publisher GetStringFileInfo(Exe, "CompanyName")
#define Copyright GetStringFileInfo(Exe, "LegalCopyright")

[Setup]
; Identifies Juku's installation across versions: never change it.
AppId={{D1D92E35-2F0C-4F9B-A0AB-D555AD4B577D}
AppName=Juku
AppVersion={#Version}
AppVerName=Juku {#Version}
AppPublisher={#Publisher}
AppCopyright={#Copyright}
VersionInfoVersion={#GetVersionNumbersString(Exe)}
PrivilegesRequired=lowest
DefaultDirName={autopf}\Juku
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
SetupIconFile=..\resources\Juku.ico
; Juku's artwork in place of Inno Setup's (resources\make_artwork.ps1): the
; icon at the top right of each page and a side panel on the last one, each
; in sizes for 100% to 250%, of which Setup takes the best fit.
WizardSmallImageFile=WizardSmallImage-58.png,WizardSmallImage-72.png,WizardSmallImage-87.png,WizardSmallImage-116.png,WizardSmallImage-145.png
WizardImageFile=WizardImage-202x386.png,WizardImage-252x482.png,WizardImage-303x579.png,WizardImage-404x772.png,WizardImage-505x965.png
UninstallDisplayIcon={app}\Juku.exe
UninstallDisplayName=Juku
OutputBaseFilename=Juku-{#Version}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
; Juku and Explorer's thumbnail process may hold the files being replaced.
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
english.FileTypes=Open .juk disk images and .jukusnap snapshots in Juku, and show snapshots' pictures in Explorer
english.Shortcuts=Shortcuts:

[Tasks]
Name: "filetypes"; Description: "{cm:FileTypes}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:Shortcuts}"; Flags: unchecked

[Files]
Source: "{#Source}\Juku.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\JukuThumbnail.dll"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Juku"; Filename: "{app}\Juku.exe"
Name: "{autodesktop}\Juku"; Filename: "{app}\Juku.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Juku.exe"; Parameters: "--register"; Tasks: filetypes; Flags: runhidden
Filename: "{app}\Juku.exe"; Description: "{cm:LaunchProgram,Juku}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\Juku.exe"; Parameters: "--unregister"; Flags: runhidden; RunOnceId: "UnregisterFileTypes"
