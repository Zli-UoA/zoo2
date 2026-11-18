; zoo2 デスクトップ版 (Tauri) の Windows 用 Inno Setup インストーラー定義。
; ビルド例:
;   iscc /DAppVersion=0.2.12 desktop-tauri\installer\zoo2.iss

#if Ver < EncodeVer(6, 3, 0)
  #error This script requires Inno Setup 6.3.0 or later for UTF-8 without BOM.
#endif

#ifndef AppVersion
  #define AppVersion "0.2.12"
#endif
#ifndef ExePath
  #define ExePath "..\src-tauri\target\release\zoo2-desktop-tauri.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\src-tauri\target\release\bundle\inno"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "zoo2_" + AppVersion + "_x64-setup"
#endif

[Setup]
AppId={{8E9B3B8E-3B89-4F0F-B7C5-6E4F1D79A2C1}
AppName=zoo2
AppVersion={#AppVersion}
AppVerName=zoo2 {#AppVersion}
AppPublisher=Zli
AppPublisherURL=https://github.com/Zli-UoA/zoo2
AppSupportURL=https://github.com/Zli-UoA/zoo2
AppUpdatesURL=https://github.com/Zli-UoA/zoo2/releases
DefaultDirName={userpf}\zoo2
DefaultGroupName=zoo2
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; tauri-plugin-single-instance が作成するミューテックス名 ({identifier}-sim)。
AppMutex=chat.zli.desktop-sim
CloseApplications=yes
CloseApplicationsFilter=*.exe
; 自動更新時の再起動は /RELAUNCH=1 と [Run] で確実に行う。
RestartApplications=no
SetupIconFile=..\src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\zoo2-desktop-tauri.exe
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=auto

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
japanese.WebView2ExecFailed=Microsoft Edge WebView2 Runtime のインストーラーを起動できませんでした。
japanese.WebView2InstallFailed=Microsoft Edge WebView2 Runtime のインストールに失敗しました (終了コード: %d)。
japanese.WebView2DownloadFailed=Microsoft Edge WebView2 Runtime が見つからず、自動ダウンロードにも失敗しました。ネットワーク接続を確認してやり直してください。
japanese.RemoveUserDataPrompt=設定やログイン情報、キャッシュデータ (%APPDATA%\zoo2-desktop-tauri、%LOCALAPPDATA%\chat.zli.desktop) も削除しますか？
japanese.RemoveUserDataHint=「いいえ」を選ぶと、再インストール時に設定とログイン状態を引き継ぎます。
english.WebView2ExecFailed=Failed to start the Microsoft Edge WebView2 Runtime installer.
english.WebView2InstallFailed=Failed to install Microsoft Edge WebView2 Runtime (exit code: %d).
english.WebView2DownloadFailed=Microsoft Edge WebView2 Runtime was not found and automatic download failed. Check your network connection and try again.
english.RemoveUserDataPrompt=Do you also want to delete your settings, sign-in session, and cached data (%APPDATA%\zoo2-desktop-tauri, %LOCALAPPDATA%\chat.zli.desktop)?
english.RemoveUserDataHint=Choose No to keep your settings and sign-in state if you reinstall zoo2 later.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; DestName: "zoo2-desktop-tauri.exe"; Flags: ignoreversion

[Icons]
; AppUserModelID を Tauri 側の WINDOWS_APP_ID と揃え、Windows トースト通知が zoo2 として出るようにする。
Name: "{userprograms}\zoo2"; Filename: "{app}\zoo2-desktop-tauri.exe"; AppUserModelID: "chat.zli.desktop"
Name: "{userdesktop}\zoo2"; Filename: "{app}\zoo2-desktop-tauri.exe"; Tasks: desktopicon; AppUserModelID: "chat.zli.desktop"

[Run]
Filename: "{app}\zoo2-desktop-tauri.exe"; Description: "{cm:LaunchProgram,zoo2}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\zoo2-desktop-tauri.exe"; Flags: nowait; Check: ShouldRelaunchAfterUpdate

[Code]
const
  SYNCHRONIZE = $00100000;
  APP_MUTEX_NAME = 'chat.zli.desktop-sim';
  APP_AUMID_REGKEY = 'Software\Classes\AppUserModelId\chat.zli.desktop';
  WEBVIEW2_CLIENT_GUID = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WEBVIEW2_BOOTSTRAPPER_URL = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703';

function OpenProcess(dwDesiredAccess: LongWord; bInheritHandle: BOOL; dwProcessId: LongWord): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(hHandle: THandle; dwMilliseconds: LongWord): LongWord;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(hObject: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

function InitializeSetup(): Boolean;
var
  WaitPidStr: String;
  WaitPid: LongWord;
  ProcessHandle: THandle;
  ElapsedMs: Integer;
begin
  WaitPidStr := ExpandConstant('{param:WAITPID|0}');
  WaitPid := StrToIntDef(WaitPidStr, 0);
  if WaitPid > 0 then begin
    ProcessHandle := OpenProcess(SYNCHRONIZE, False, WaitPid);
    if ProcessHandle <> 0 then begin
      try
        WaitForSingleObject(ProcessHandle, 15000);
      finally
        CloseHandle(ProcessHandle);
      end;
    end;
    ElapsedMs := 0;
    while CheckForMutexes(APP_MUTEX_NAME) and (ElapsedMs < 15000) do begin
      Sleep(100);
      ElapsedMs := ElapsedMs + 100;
    end;
  end;
  Result := True;
end;

function ShouldRelaunchAfterUpdate(): Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

function HasWebView2Key(RootKey: Integer; const SubKey: String): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(RootKey, SubKey, 'pv', Version) and
    (Version <> '') and (Version <> '0.0.0.0');
end;

function IsWebView2Installed(): Boolean;
begin
  Result :=
    HasWebView2Key(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\' + WEBVIEW2_CLIENT_GUID) or
    HasWebView2Key(HKLM, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\' + WEBVIEW2_CLIENT_GUID) or
    HasWebView2Key(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\' + WEBVIEW2_CLIENT_GUID);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  BootstrapperPath: String;
  ResultCode: Integer;
begin
  Result := '';
  if IsWebView2Installed() then
    Exit;

  try
    DownloadTemporaryFile(WEBVIEW2_BOOTSTRAPPER_URL, 'MicrosoftEdgeWebView2Setup.exe', '', nil);
    BootstrapperPath := ExpandConstant('{tmp}\MicrosoftEdgeWebView2Setup.exe');
    if not Exec(BootstrapperPath, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then begin
      Result := CustomMessage('WebView2ExecFailed');
      Exit;
    end;
    if (ResultCode <> 0) and (not IsWebView2Installed()) then begin
      Result := Format(CustomMessage('WebView2InstallFailed'), [ResultCode]);
      Exit;
    end;
  except
    Result := CustomMessage('WebView2DownloadFailed');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ConfigDir: String;
  LocalDataDir: String;
  PromptText: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  RegDeleteKeyIncludingSubkeys(HKCU, APP_AUMID_REGKEY);

  ConfigDir := ExpandConstant('{userappdata}\zoo2-desktop-tauri');
  LocalDataDir := ExpandConstant('{localappdata}\chat.zli.desktop');
  if (not UninstallSilent) and (DirExists(ConfigDir) or DirExists(LocalDataDir)) then begin
    PromptText := CustomMessage('RemoveUserDataPrompt') + #13#10 + #13#10 + CustomMessage('RemoveUserDataHint');
    if SuppressibleMsgBox(PromptText, mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then begin
      if DirExists(ConfigDir) then
        DelTree(ConfigDir, True, True, True);
      if DirExists(LocalDataDir) then
        DelTree(LocalDataDir, True, True, True);
    end;
  end;
end;
