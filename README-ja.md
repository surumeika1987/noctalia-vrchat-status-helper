# VRChat Status Helper
Noctaliaプラグイン `VRChat Status`とデータをやり取りするヘルパーソフトウェアです。  
`vrchatapi` クレートを利用して**非公式**VRChat APIと通信します。  

## 注意
VRChat APIは**非公式**です。  
本ソフトウェアを利用することで発生するいかなる問題も開発者は責任を負いません。  
本ソフトウェアの利用は自己責任で行なってください。  

## インストール
以下のコマンドを実行してダウンロード、インストールしてください。  
```sh
mkdir -p ~/.local/bin
wget -O ~/.local/bin/vrchat-status-helper https://github.com/surumeika1987/noctalia-vrchat-status-helper/releases/download/v0.1.0/vrchat-status-helper-x86_64-unknown-linux-gnu
chmod +x ~/.local/bin/vrchat-status-helper
```

## ビルド
Rustをビルドできる環境が必要です。  
以下のコマンドを実行してビルド、インストールしてください。  
```sh
git clone https://github.com/surumeika1987/noctalia-vrchat-status-helper.git
cd noctalia-vrchat-status-helper
cargo build --release
mkdir -p ~/.local/bin
cp ./target/release/vrchat-status-helper ~/.local/bin/
```

## 使用方法
Noctaliaプラグイン `VRChat Status` と合わせて利用してください。  
以下のコマンドで手動インストールできます。  
```
git clone https://github.com/surumeika1987/noctalia-vrchat-status.git \
    ~/.local/share/noctalia/plugins/vrchat-status
```

### ログイン
以下のコマンドを利用してVRChatにログインしてください。  
初回起動時と認証情報の期限切れの場合に設定が必要です。  
```sh
vrchat-status-helper login
```
認証情報は`$XDG_CACHE_HOME/noctalia/vrchat-status/cookies.txt`又は  
`~/.cache/noctalia/vrchat-status/cookies.txt`に権限`0600`で保存されます。  
認証情報の取り扱いには十分注意してください。  

### 起動方法
`vrchat-status-helper`を引数なしで起動した場合、常駐ソフトウェアとして起動します。  
Hyprland等のWMに自動起動する設定を追加することをおすすめします。  
```lua
hl.on("hyprland.start", function()
    hl.exec_cmd("noctalia")
    hl.exec_cmd("/home/<your name>/.local/bin/vrchat-status-helper")
end)
```

### VRChat APIへのアクセスについて
VRChat APIへの負荷軽減を目的として、  
本ソフトウェアでは60秒間あたり1回のアクセス制限をかけています。  
そのため、変更内容がVRChat側に反映されるまで時間がかかる場合があります。  

### テストモード
VRChat APIへ接続せずにプラグインをテストする場合は、`test`引数だけを指定して
起動します。
```sh
vrchat-status-helper test
```
テストモードは`4:Test Mode`を初期状態として起動します。`push-status` IPCを
受信すると内部ステータスを更新し、変更後のステータスを直ちにNoctaliaプラグインへ
送信します。

### 開発者向け
本ソフトウェアはUnixソケットによるIPC通信を受け付けています。  
以下のコマンドでIPC通信を行うことができます。  
```sh
vrchat-status-helper msg push-status <payload>
```
ペイロードのフォーマットは `<ステータス番号>:<ステータスメッセージ>`です。  
ステータスメッセージは空白にすることができます。  
ステータス番号は以下のとおりです。  

| 番号 | VRChat上のステータス名 |
| --- | --- |
| 4 | Join Me |
| 3 | Online |
| 2 | Ask Me |
| 1 | Do Not Disturb |
| 0 | Offline |

常駐中のhelperにキャッシュ済みステータス（キャッシュがない場合は`Need Login`）を
再送させるには、以下のコマンドを使用します。

```sh
vrchat-status-helper msg request-push
```

起動時に`RUST_LOG=debug`を付与することでデバッグログを出力できます。  
```sh
RUST_LOG=debug vrchat-status-helper
```
