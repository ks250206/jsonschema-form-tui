# jsonschema-form-tui

JSON Schema からフォームを生成し、TUI 上で安全に JSON を編集する Rust 製ツールです。

狙いは、`react-json-schema-form` のような schema-driven form を、`lazygit` ライクな分割ペイン UI と Vim 風操作で扱えるようにすることです。

## 機能

- JSON Schema 2020-12 ベースの JSON 入力
- schema から form を自動生成
- `enum` / `boolean` の select 表示
- `textarea` 形式の複数行入力
- array の追加・削除
- object array を含む nested form 表示
- jq 風の簡易 filter
- pretty JSON 出力
- 出力ファイル保存と上書き確認
- editor / standard の 2 モード
- Vim 風キーバインド、Tab 移動、マウス操作（クリック・ホイール）
- `?` でキーヘルプオーバーレイ（セクション見出し・キーキャップ付き）
- 2 行フッター（ステータス行 + コンテキスト別ショートカット行、端末上は上ボーダー含めて高さ 3 行相当を使用）
- フォームの深いネストでも見切れ時にヒント表示・スクロール可能なレイアウト
- ログにフォーム確定時の `旧値 -> 新値` 形式の記録（スカラー・`oneOf` 枝切替など）

## スクリーンショット的な構成

`editor` モード:

- 上段: `Schema Path`
- 左上: `Schema`
- 左下: `Form`
- 右上: `Output Path`
- 右中: `Filter`
- 右下: `Output`
- 下段: `Log`
- 最下段: `Footer`（上ボーダー + 2 行テキスト）

`standard` モード:

- `Schema` ペインを隠した利用者向けレイアウト
- `Schema Path` は read-only

## セットアップ

```bash
cargo run
```

主な引数:

```bash
cargo run -- --mode standard
cargo run -- --mode editor
cargo run -- --schema ./schema/basic.json
cargo run -- --schema ./schema/basic.json --output ./output.json
```

CLI オプション:

- `--mode standard|editor`
- `--schema <PATH>`
- `--output <PATH>`

## サンプル schema

`schema/` 配下にサンプルを置いています。

- `./schema/basic.json`
- `./schema/profile.json`
- `./schema/deploy.json`
- `./schema/enum-array.json`
- `./schema/material.json`（ネスト・`oneOf` の確認用）

`basic.json` には以下の例が入っています。

- 文字列
- number / integer
- boolean
- enum
- textarea
- string array
- object array

## モード

### Standard

- デフォルト起動モード
- form 入力中心
- `Schema` ペインは非表示
- `Schema Path` は参照のみ

### Editor

- schema 自体を編集したいときのモード
- `Schema` ペインを表示
- `Schema Path` を編集可能
- path candidate footer を表示

## 基本操作

### ペイン移動

- `1-7`（`standard` では `1-6`）: pane focus（番号はモードでずれる）
- `Tab` / `Shift-Tab`: pane focus を巡回
- マウスクリック: pane focus

### ヘルプ

- `?`: キーボードリファレンスオーバーレイ（`Esc` または `?` で閉じる）

### マウス

- クリック: ペインフォーカス・フォームのフィールド / 配列ボタン
- ホイール: `Schema` / `Output` / `Log` では 1 ノッチあたり 1 行相当でスクロール
- ホイール: `Form` では感度を下げており、複数ノッチで 1 フィールド分移動（細かいホイールでも飛びにくい）

### 移動

- `h j k l`
- 矢印キー
- `0` / `$`
- `w` / `b` / `e`
- `gg` / `G`

### モード

- `i`: 現在位置の前で insert
- `a`: 現在位置の後で insert
- `v`: visual
- `Esc`: normal に戻る

### 編集

- `x` / `Delete`: 1 文字削除
- `D` / `d$`: 行末まで削除
- `d0`: 行頭まで削除
- `dw`: 次単語境界まで削除
- `dd`: 行削除
- `o` / `O`: 行挿入
- `u`: undo
- `Ctrl-r`: redo

### Form

- `Enter`: single-line field の編集 on/off、select button 実行
- `Tab` / `Shift-Tab`: insert 中は field / array button 間移動
- `h` / `l` / 左右矢印: select 値切替
- `+`: array item を追加
- `-`: array item を削除
- `R`: form 全体を schema default に戻す

### Schema Path

- `Tab`: candidate を順送り補完
- `Shift-Tab`: candidate を逆送り補完
- `Enter`: commit

### Schema

- 行番号表示あり
- `Tab`: 4 spaces indent
- `Shift-Tab`: outdent
- JSON / JSON Schema として不正なら pane 下端と log にエラー表示

### Output

- 行番号ガターあり（`Schema` と同形式）
- タイトルに `(current/total)` の行進捗（`Schema` / `Log` と同様）
- `r`: `Output Path` に保存
- 既存ファイルの場合は overwrite popup を表示
- `y` / `Enter`: 上書き
- `n` / `Esc`: キャンセル

### Fold

- `za`: pane fold toggle
- `zc`: pane collapse
- `zo`: pane expand

対象:

- `Schema`
- `Form`

### メインエリア full width

`Schema Path` と `Log`（およびフッター）はそのまま、中央帯だけを次のいずれかで全幅表示するトグル（`za` と同様に `z` の後続キー）。

- `z` `w`: アクティブペインに応じて切替 / 解除
  - `Schema` フォーカス時: `Schema` のみ全幅（`Form` と右列を隠す）
  - `Form` フォーカス時: `Form` のみ全幅
  - `Output Path` / `Filter` / `Output` フォーカス時: 右列（Path + Filter + Output）を全幅

もう一度同じ組み合わせで解除する。

## Form 表示の仕様

- `description` は field label の下に表示
- `enum` / `boolean` は select として表示
- `format: "textarea"` は複数行入力として表示
- nested `object` は枠付きで折りたたみ可能なブロックとして表示
- ペイン内に収まらない場合は枠のクリップや「続きあり（フォーム内カーソルを下へ）」等のヒントで、下にフィールドがあることが分かるようにする
- array は group 枠を 1 段追加して表示
- variable-length array は `Add Item` / `Remove Item` button を表示
- breadcrumb を `Form` 上部に表示（1 行）
- フォーム内の「行」カーソル（`j` / `k` 等）で先頭フィールドからのスクロール相当の表示オフセットを変えられる

## Validation

- form の値は on-blur 相当で検証
- invalid value は field error と log に表示
- invalid schema は最後の正常な schema を保持
- schema parse / schema validation error は `Schema` ペイン下端に表示

## JSON Schema 対応状況

このアプリでは、`jsonschema` crate による validation と、TUI 上での form 生成 / 編集 UI を分けて考えています。

- validation: crate 側でかなり広く対応
- form 生成 / UI: このアプリ側で明示実装した範囲だけ対応

### 実装済み

| 項目 | 状態 | メモ |
| --- | --- | --- |
| `type: object` | 対応 | `properties` から form を生成 |
| `type: string` | 対応 | 通常 textbox |
| `type: integer` / `number` | 対応 | 数値入力制限あり |
| `type: boolean` | 対応 | select 表示 |
| `enum` | 対応 | select 表示 |
| `const` | 対応 | default 生成で利用 |
| `default` | 対応 | 初期値生成に利用 |
| `$ref` | 対応 | internal ref のみ |
| `required` | 対応 | label に反映 |
| `description` | 対応 | field 補助文として表示 |
| `format: "textarea"` | 対応 | multiline editor |
| `pattern` / `minLength` / `maxLength` | 対応 | validation と説明表示 |
| `minimum` / `maximum` / `exclusiveMinimum` / `exclusiveMaximum` | 対応 | validation と説明表示 |
| `array` | 対応 | scalar / object array を form 展開 |
| `minItems` / `maxItems` | 対応 | add/remove 制御に反映 |
| `items` | 対応 | 可変長 array の item schema に利用 |
| `prefixItems` | 対応 | tuple array の default / form 展開に利用 |

### 未対応または部分対応

| 項目 | 状態 | メモ |
| --- | --- | --- |
| `oneOf` | 部分対応 | Form に `(oneOf)` 行を出し、`h`/`l`（循環）または先頭一致のキーで枝切替。切替時はそのサブツリーを枝の default で置換 |
| `anyOf` / `allOf` | 未対応 | form 切替 UI なし |
| `if` / `then` / `else` | 未対応 | 条件付き form 再構築なし |
| `dependentSchemas` / `dependentRequired` | 未対応 | UI 上の依存制御なし |
| `patternProperties` | 未対応 | 動的キー編集 UI なし |
| `additionalProperties` | 未対応 | 任意 key/value 追加 UI なし |
| `unevaluatedProperties` / `unevaluatedItems` | 未対応 | UI 制御なし |
| `contains` / `minContains` / `maxContains` | 未対応 | array 編集 UI では未考慮 |
| `uniqueItems` | validation のみ | 専用 UI フィードバックなし |
| `format` 一般 | 部分対応 | `textarea` 以外は validation 依存で専用 UI なし |
| 外部 `$ref` | 未対応 | ローカル / URL ref 解決なし |
| null を含む union type | 未対応 | 例: `type: ["string", "null"]` の UI 未対応 |
| tuple array の item ごとの追加削除制御 | 部分対応 | `prefixItems` 展開はするが高度な編集 UI は未対応 |

## 出力と filter

- `Output` は 4 spaces indent の pretty JSON
- `Filter` は簡易 jq 風
- validation summary は `Output` ペイン下端の 1 行フッターに表示（本文の行番号とは別）

## ログ

- 行番号付き表示（`NNNN | メッセージ` 形式）
- 自動で末尾に追従
- タイトルに `(current/total)` を表示
- フォームでスカラーを確定したとき: `form field <path>: <old> -> <new>`（長い値は省略）
- `oneOf` 枝変更: `form oneOf <path>: branch index <old> -> <new>`

## 既知の前提

- 本格的な jq 全機能ではなく、簡易 filter のみ
- pane fold は pane 全体の縮退であり、AST / field 単位の fold ではない
- `editor` モードでは schema 編集時に valid な JSON Schema へ戻した時点で form を再構築する

## テスト

```bash
cargo test
```

現在の主なテスト対象:

- schema default 生成
- form field 展開（`oneOf` 先頭枝・内部 `$ref` など）
- enum / textarea / array
- save / overwrite
- undo / redo
- cursor / scroll（ペイン・フォームパーティション・マウスホイール）
- invalid schema / invalid form value
- UI: `material.json` を用いた描画スモークテスト
