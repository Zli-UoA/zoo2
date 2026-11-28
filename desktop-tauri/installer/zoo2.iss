; zoo2 デスクトップ版 (Tauri) の Windows 用 Inno Setup インストーラー定義。
; ビルド例:
;   iscc /DAppVersion=0.3.0 desktop-tauri\installer\zoo2.iss

#if Ver < EncodeVer(6, 5, 2)
  #error This script requires Inno Setup 6.5.2 or later for UTF-8 without BOM and PNG wizard images.
#endif

#ifndef AppVersion
  #define AppVersion "0.4.1"
#endif
; プレリリース版 (例: 0.3.1-rc.1) でも数値専用の VersionInfo 項目へ渡せるように、
; ハイフン以降を除いた数値部分を AppVersionNumeric として定義する。
#if Pos("+", AppVersion) != 0
  #define AppVersionWithoutBuild Copy(AppVersion, 1, Pos("+", AppVersion) - 1)
#else
  #define AppVersionWithoutBuild AppVersion
#endif
#if Pos("-", AppVersionWithoutBuild) != 0
  #define AppVersionNumeric Copy(AppVersionWithoutBuild, 1, Pos("-", AppVersionWithoutBuild) - 1)
#else
  #define AppVersionNumeric AppVersionWithoutBuild
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
; インストーラー自身のファイル情報にも版を入れ、OS 側のアプリ一覧やファイルのプロパティから版を確認できるようにする。
VersionInfoVersion={#AppVersionNumeric}
VersionInfoProductVersion={#AppVersionNumeric}
VersionInfoProductName=zoo2
VersionInfoCompany=Zli
; 完全な版 (プレリリースを含む) はテキスト項目に残す。
VersionInfoProductTextVersion={#AppVersion}
VersionInfoTextVersion={#AppVersion}
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
WizardImageFile=wizard-image.png
WizardImageBackColor=$00D6FF
WizardSmallImageFile=wizard-small.png
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
japanese.WebView2VerifyFailed=Microsoft Edge WebView2 Runtime インストーラーの署名を確認できませんでした。
japanese.WebView2ExecFailed=Microsoft Edge WebView2 Runtime のインストーラーを起動できませんでした。
japanese.WebView2InstallFailed=Microsoft Edge WebView2 Runtime のインストールに失敗しました (終了コード: %d)。
japanese.WebView2DownloadFailed=Microsoft Edge WebView2 Runtime が見つからず、自動ダウンロードにも失敗しました。ネットワーク接続を確認してやり直してください。
japanese.RemoveUserDataPrompt=設定やログイン情報、キャッシュデータ (%APPDATA%\zoo2-desktop-tauri、%LOCALAPPDATA%\chat.zli.desktop) も削除しますか？
japanese.RemoveUserDataHint=「いいえ」を選ぶと、再インストール時に設定とログイン状態を引き継ぎます。
japanese.RemoveUserDataFailed=一部の設定・データフォルダを削除できませんでした。アプリが終了しているか確認し、必要に応じて手動で削除してください:%n%n%s
english.WebView2VerifyFailed=Failed to verify the digital signature of the Microsoft Edge WebView2 Runtime installer.
english.WebView2ExecFailed=Failed to start the Microsoft Edge WebView2 Runtime installer.
english.WebView2InstallFailed=Failed to install Microsoft Edge WebView2 Runtime (exit code: %d).
english.WebView2DownloadFailed=Microsoft Edge WebView2 Runtime was not found and automatic download failed. Check your network connection and try again.
english.RemoveUserDataPrompt=Do you also want to delete your settings, sign-in session, and cached data (%APPDATA%\zoo2-desktop-tauri, %LOCALAPPDATA%\chat.zli.desktop)?
english.RemoveUserDataHint=Choose No to keep your settings and sign-in state if you reinstall zoo2 later.
english.RemoveUserDataFailed=Could not delete some settings or data folders. Make sure zoo2 is closed and remove them manually if needed:%n%n%s

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
function SetEnvironmentVariable(lpName, lpValue: String): BOOL;
  external 'SetEnvironmentVariableW@kernel32.dll stdcall';

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

function VerifyMicrosoftSignature(const FilePath: String): Boolean;
var
  PsExe: String;
  Cmd: String;
  ResultCode: Integer;
begin
  Result := False;
  PsExe := ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe');
  if not FileExists(PsExe) then
    PsExe := 'powershell.exe';
  // U+2019 (’) 等の引用符解釈を避けるため、対象パスはコードへ埋め込まず環境変数で渡す。
  if not SetEnvironmentVariable('ZOO2_WV2_INSTALLER', FilePath) then
    Exit;
  try
    Cmd :=
      '-NoProfile -NonInteractive -WindowStyle Hidden -Command "' +
      '$p = $env:ZOO2_WV2_INSTALLER; if (-not $p) { exit 1 }; ' +
      '$s = Get-AuthenticodeSignature -LiteralPath $p; ' +
      'if ($s.Status -eq ''Valid'' -and $s.SignerCertificate -and ' +
      '($s.SignerCertificate.Subject -match ''(^|,)\s*O=\x22?Microsoft Corporation\x22?\s*(,|$)'')) ' +
      '{ exit 0 } else { exit 1 }"';
    if Exec(PsExe, Cmd, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      Result := (ResultCode = 0);
  finally
    SetEnvironmentVariable('ZOO2_WV2_INSTALLER', '');
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  BootstrapperPath: String;
  ResultCode: Integer;
  ElapsedMs: Integer;
begin
  Result := '';
  if IsWebView2Installed() then
    Exit;

  try
    DownloadTemporaryFile(WEBVIEW2_BOOTSTRAPPER_URL, 'MicrosoftEdgeWebView2Setup.exe', '', nil);
    BootstrapperPath := ExpandConstant('{tmp}\MicrosoftEdgeWebView2Setup.exe');
  except
    Result := CustomMessage('WebView2DownloadFailed');
    Exit;
  end;

  try
    if not VerifyMicrosoftSignature(BootstrapperPath) then begin
      Result := CustomMessage('WebView2VerifyFailed');
      Exit;
    end;
    if not Exec(BootstrapperPath, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then begin
      Result := CustomMessage('WebView2ExecFailed');
      Exit;
    end;
    ElapsedMs := 0;
    while (not IsWebView2Installed()) and (ElapsedMs < 5000) do begin
      Sleep(500);
      ElapsedMs := ElapsedMs + 500;
    end;
    if not IsWebView2Installed() then begin
      Result := Format(CustomMessage('WebView2InstallFailed'), [ResultCode]);
      Exit;
    end;
  finally
    DeleteFile(BootstrapperPath);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ConfigDir: String;
  LocalDataDir: String;
  FallbackWebViewDir: String;
  PromptText: String;
  FailedDirs: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  RegDeleteKeyIncludingSubkeys(HKCU, APP_AUMID_REGKEY);

  ConfigDir := ExpandConstant('{userappdata}\zoo2-desktop-tauri');
  LocalDataDir := ExpandConstant('{localappdata}\chat.zli.desktop');
  FallbackWebViewDir := ExpandConstant('{app}\zoo2-desktop-tauri.exe.WebView2');
  if (not UninstallSilent) and (DirExists(ConfigDir) or DirExists(LocalDataDir) or DirExists(FallbackWebViewDir)) then begin
    PromptText := CustomMessage('RemoveUserDataPrompt') + #13#10 + #13#10 + CustomMessage('RemoveUserDataHint');
    if SuppressibleMsgBox(PromptText, mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then begin
      FailedDirs := '';
      if DirExists(ConfigDir) and (not DelTree(ConfigDir, True, True, True)) then
        FailedDirs := ConfigDir;
      if DirExists(LocalDataDir) and (not DelTree(LocalDataDir, True, True, True)) then begin
        if FailedDirs <> '' then
          FailedDirs := FailedDirs + #13#10;
        FailedDirs := FailedDirs + LocalDataDir;
      end;
      if DirExists(FallbackWebViewDir) and (not DelTree(FallbackWebViewDir, True, True, True)) then begin
        if FailedDirs <> '' then
          FailedDirs := FailedDirs + #13#10;
        FailedDirs := FailedDirs + FallbackWebViewDir;
      end;
      if FailedDirs <> '' then
        SuppressibleMsgBox(Format(CustomMessage('RemoveUserDataFailed'), [FailedDirs]), mbError, MB_OK, IDOK);
    end;
  end;
end;
