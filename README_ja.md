# ProcSight

[English](README.md) | 日本語

Windows 11 の標準のタスク マネージャーの代わりに使う、日本語のプロセス管理ソフトです。Rust と egui で作った 1 つの exe で、インストールは要りません。

![プロセスのタブ](docs/images/s1-processes-dark.png)

## できること

- **プロセス**: ツリーと一覧、並べ替え、名前・PID・パスでの絞り込み、列の表示・順序・幅の変更(次の起動にも残る)、新しく始まった行と終わった行の色分け
- **操作**: 終了、ツリーごとの終了、複数まとめての終了、優先度(低〜高)、CPU の割り当て、効率モード、一時停止と再開、前面に出す、ファイルの場所を開く、プロパティ
- **詳細**: 全般(パス・コマンドライン・親・ユーザー・昇格・整合性レベル・アーキテクチャ)、スレッド、DLL、ハンドル、署名(ネットにつながずに確かめる)、CPU とメモリの小さなグラフ
- **ファイルを掴んでいるプロセスを探す**: 「使用中で消せない」ファイルをドロップすると、使っているプロセスが分かる
- **パフォーマンス**: CPU(全体と論理プロセッサごと)、メモリ、ディスク、ネットワーク、GPU のグラフ(直近 60 回分)
- **スタートアップ**: レジストリの Run とスタートアップ フォルダの項目を有効・無効にする(Windows のタスク マネージャーと同じ記録の仕方なので、どちらで切り替えても同じ状態が出る)
- **サービス**: 開始・停止・再起動・開始の種類の変更。止める前に、動いている依存のサービスを一覧で確かめる
- **Ctrl+Shift+Esc の置き換え**(設定でオン。既定はオフ): Ctrl+Shift+Esc と Ctrl+Alt+Del の「タスク マネージャー」で ProcSight が開く
- ダークとライトのテーマ、文字の大きさ、トレイのアイコン(設定でオン)、キーボードの操作(Delete・Ctrl+F・Ctrl+1〜4・Ctrl+C)

![CPU のグラフ](docs/images/s2-cpu-cores-dark.png)

![ハンドルの一覧](docs/images/s1-detail-handles-dark.png)

## ProcSight がしないこと

- **ネットにつながりません。** 更新の確認・テレメトリ・オンラインの検索・署名の失効の確認を含め、通信は一切しません。
- **履歴を保存しません。** グラフの値はメモリの直近 60 回分だけです。
- **大事なプロセスは止めません。** System・csrss.exe・lsass.exe などの Windows の中核のプロセス、重要プロセスの印があるプロセス、ProcSight 自身は、終了も一時停止もできないようにしてあります。
- 優先度「リアルタイム」、ダンプの作成、他のプロセスのハンドルを閉じる・メモリを読み書きする機能はありません。

## 触れないプロセス(ゲームを遊ぶ人へ)

設定の「触れないプロセス」に exe 名(例: `game.exe`)を登録すると、そのプロセスと子孫には ProcSight からハンドルを開きません。一覧には出ますが、詳細の取得と操作は行いません。ゲームのアンチチートに怪しまれないようにするための設定です。

![触れないプロセスの設定](docs/images/s5-settings-touchfree-light.png)

- **登録していないプロセスは、パスやユーザーを読むために、見つけたときに 1 回ずつ開きます**(読むだけの権限で開いて、すぐ閉じます)。ゲームは、ProcSight を起動する前、またはゲームを起動する前に登録してください。
- 一覧の取得は、どのプロセスのハンドルも開かない方法(`NtQuerySystemInformation`)だけで行います。
- ハンドルを開く処理は 1 か所の関門を通り、触れないプロセスはそこで必ず断ります。開いた回数は設定の「診断」で見られます。
- **これでアンチチートに対して安全だと保証するものではありません。** 心配なときは、ゲームの間は ProcSight を閉じてください。

## 動作環境

- Windows 11(x64)。Windows 10 は対象にしていません。
- 管理者でなくても起動できます。他のユーザーのプロセス・サービス・全ユーザーのスタートアップを変えるときは、画面上部の「管理者で再起動」を使います。

## 使い方

1. [Releases](https://github.com/takosasi-dev/procsight/releases) から `procsight-v<版>-windows-x64.zip` を取り、好きなフォルダに展開します(v0.1.0 はプレリリースです。下の「開発の状況」を見てください)。
2. `ProcSight.exe` を起動します。

設定は `%APPDATA%\ProcSight\config.toml`、ログは `%LOCALAPPDATA%\ProcSight\logs\`(7 日分)に置きます。ログにコマンドライン・フルパス・ユーザー名は書きません。

### Ctrl+Shift+Esc で ProcSight を開く

Windows の Image File Execution Options(`taskmgr.exe` の `Debugger` の値)を使います。Process Explorer の「Replace Task Manager」と同じ仕組みです。

1. `ProcSight.exe` を `C:\Program Files\ProcSight\` に置きます(標準のユーザーが書き換えられない場所に置いたときだけオンにできます)。
2. ProcSight を管理者で起動し、設定の「全般」で「Ctrl+Shift+Esc で ProcSight を開く」をオンにします。
3. 本物のタスク マネージャーは、プロセスの右クリックの「Windows のタスク マネージャーを開く」(触れないプロセスの行)から開けます。

**元に戻す方法**(ProcSight が起動できなくなったときも戻せるように 3 つ用意しています):

- 設定の画面でオフにする
- 配布物の `restore-taskmgr.reg` をダブルクリックして取り込む
- 管理者のコマンド プロンプトで次を打つ

```
reg delete "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe" /v Debugger /f
```

この場所はマルウェアにも使われるため、セキュリティ ソフトが値を消すことがあります。消されたときは、ProcSight が起動時に知らせます。

## ソースからビルドする

Rust 1.95 以上と、Visual Studio の「C++ によるデスクトップ開発」(MSVC と Windows SDK)が要ります。

```
cargo build --release -p procsight-app
```

`target\release\ProcSight.exe` ができます。配布用の zip は `tools\publish.ps1` で作ります。

### 確かめ

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check licenses bans
```

画面のテストは窓を作らずに描きます(egui_kittest)。`procsight-testee` はテスト用のプロセスで、配布物には入れません。`procsight-probe` は、公開されていない API の値を公開 API と突き合わせる確かめ用の道具です。

## 構成

| crate | 役割 |
|---|---|
| `procsight-core` | Windows に依存しない判定(差分・ツリー・絞り込み・止めてよいかの判定・設定)。Windows の crate を使わない |
| `procsight-win` | Win32 と NT の API の呼び出し。`unsafe` はこの crate だけ |
| `procsight-app` | 画面(eframe / egui) |
| `procsight-probe` / `procsight-uiprobe` / `procsight-testee` | 確かめ用の道具 |

## 開発の状況

v0.1.0 はプレリリースです。自動のテスト(130 件ほど)と、標準の権限での確かめは済んでいます。画面の動きは窓を作らないテストで確かめていて、実際の窓での手触りはまだ確かめていません。管理者での操作(サービス・全ユーザーのスタートアップ・Ctrl+Shift+Esc の置き換え)と、タスク マネージャーとの値の比べは、作者の PC での確かめを待っています。

## ライセンス

MIT License([LICENSE](LICENSE))。依存する crate のライセンスは [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) にあります。日本語の文字は Windows に入っているフォント(Yu Gothic UI など)を使い、フォントは同梱していません。
