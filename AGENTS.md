# AGENTS.md

## 目的

このプロジェクトは、**JSON Schema からフォームを生成し、TUI 上で安全に JSON を編集・保存する Rust アプリケーション**です。  
狙いは **「react-json-schema-form の lazygit 風 TUI 版」** であり、`ratatui` と `crossterm` を使って、キーボード中心で高速に扱える UI を提供します。

AI コーディングエージェントは、以下を優先してください。

- JSON Schema 駆動のフォーム編集体験を壊さない
- TUI ネイティブな操作感を維持する
- 小さく明示的でテストしやすい設計を保つ
- README と現実装の両方に整合する変更を行う

---

## 現在の実装要約

2026-04-07 時点のコードベースでは、以下がすでに実装されています。

- `standard` / `editor` の 2 モード
- 7 ペイン構成
  - `Schema Path`
  - `Schema` (`editor` のみ)
  - `Form`
  - `Output Path`
  - `Filter`
  - `Output`
  - `Log`
- 最下段 `Footer`: 上ボーダー + **2 行**（ステータス行・コンテキスト別 `KEYS` 行）。レイアウト上は **高さ 3 行** を確保（`Block` ボーダーと本文の両立のため）
- `?` による **ヘルプオーバーレイ**（セクション見出し・シアンキーキャップ・英語ベースの説明）
- `normal` / `insert` / `visual` の 3 入力モード
- Vim 風移動、Undo/Redo、Visual 選択コピー
- マウスクリックによるペインフォーカスとフォーム操作
- マウスホイール: `Schema` / `Output` / `Log` で行スクロール。`Form` は **感度低下**（複数ノッチで 1 フィールド、`form_mouse_scroll_accum`）
- JSON Schema からの default 値生成
- nested object / array / object array のフォーム生成（`ObjectFrame` パーティション、見切れ時のクリップ・ヒント行、フォーム行カーソルによる表示オフセット）
- `enum` / `boolean` の選択 UI
- `oneOf` の **部分対応**（variant 行 + `h`/`l` 循環・先頭枝 default 等。README / 未対応表と整合）
- `format: "textarea"` による複数行入力
- jq 風の最小フィルター
- pretty JSON 出力
- 保存時の overwrite 確認
- `jsonschema` crate を使った Draft 2020-12 検証
- ログ: スカラー確定時 `form field path: old -> new`、`oneOf` 切替時 `form oneOf path: branch index old -> new`（英語メッセージ）

未実装の理想像を先回りで抽象化するより、**今ある挙動を改善しやすい構造に保つ**ことを優先してください。

---

## 開発方針

### 基本方針

- **テスト駆動開発を優先**
- **struct / enum 中心で明示的に設計**
- **副作用境界を分離**
- **小さな変更を積み重ねる**
- **UI に業務ロジックを埋め込まない**
- **README と実装の乖離を広げない**

### TDD の基本順序

1. まず失敗するテストを書く
2. 最小実装で通す
3. リファクタリングする
4. 既存テストを壊さず責務を整える

### テスト優先順位

1. `domain` の純粋関数
2. `app/state` の状態遷移
3. `ui` のキーマップ・レイアウト・操作フロー
4. `infra` のファイル入出力境界

### モック方針

原則として以下は本物を使ってテストしてください。

- JSON Schema validation
- form 生成
- filter 評価
- state 遷移
- JSON 生成

モックや差し替えを許容する境界は最小限です。

- ファイルシステム
- 端末イベント
- クリップボード

---

## 技術スタック

現実装の基本スタック:

- Rust 2024 edition
- Cargo
- ratatui
- crossterm
- tui-textarea
- serde / serde_json
- jsonschema (`draft202012`)
- clap
- anyhow / thiserror
- tracing / tracing-subscriber
- arboard
- insta / tempfile

`README.md` や `Cargo.toml` に存在しないライブラリを前提に設計を書き足さないでください。  
特に現時点では `jaq` 系は未導入です。

---

## 非目標

明示的に求められない限り、以下はスコープ外です。

- ブラウザ UI 前提の抽象化
- クラウド同期
- 複数人編集
- 早すぎるプラグイン化
- jq 完全互換
- JSON Schema 全キーワードの完全 UI 対応

---

## TUI 要件

### 画面モード

現実装の `AppMode` は以下です。

- `standard`
- `editor`

`standard` は利用者向けのフォーム編集モードです。

- `Schema` ペインは非表示
- `Schema Path` は基本 read-only 運用
- `Form` と `Output` を主役にする

`editor` はスキーマ自体を編集するモードです。

- `Schema` ペインを表示
- `Schema Path` を編集可能
- スキーマ補完候補を footer に出せる

### 入力モード

現実装の `InputMode` は以下の 3 つです。

- `normal`
- `insert`
- `visual`

`field-focus` の独立モードはまだありません。  
今後追加する場合も、既存の `Form` 操作や `Tab` 移動との責務分離を明確にしてから行ってください。

### 現在のペイン構成

現在のレイアウトは以下を基準とします。

- 最上段 full-width: `Schema Path`
- 左上: `Schema` (`editor` のみ)
- 左下: `Form`
- 右上: `Output Path`
- 右中: `Filter`
- 右下: `Output`
- 下段: `Log`
- 最下段: `Footer`（端末上 **3 行**、`Constraint::Length(3)`）

各ペインタイトルには表示中レイアウトに応じた番号を付与します。

- `editor`: `[1]` から `[7]`
- `standard`: `Schema` を除いた `[1]` から `[6]`

### フォーカスとカーソル

- 各ペインは独立したカーソルを持つ
- アクティブペインは枠色で区別する
- `hjkl` / 矢印はアクティブペイン内移動
- 数字キー、`Tab`、`Shift-Tab`、マウスクリックでペイン移動
- マウスホイール: `Schema` / `Form` / `Output` / `Log` 上でスクロール可能（`ui/app.rs` の `handle_scroll`）。`Form` のみ `FORM_MOUSE_SCROLL_TICKS_PER_ROW` で感度調整
- `Schema` / `Output` / `Log` は行進捗 `(current/total)` をタイトルに表示する
- `Schema` と `Output` の本文は行番号ガター付き（マウスクリック時の列はガター幅を差し引いて計算）
- `Form` は先頭にパンくず用に **1 行** を確保（レイアウト `Length(1) | Min(1)`）。マウスヒットテストは本文エリア基準

### 折りたたみ

現実装では以下のみ対応します。

- `Schema`
- `Form`

キー:

- `za`: toggle
- `zc`: collapse
- `zo`: expand

ペイン内 AST 単位や field 単位の fold はまだ対象外です。

### メインエリア full width

- `Schema Path` / `Log` / フッターは通常レイアウトのまま
- `z` `w`: アクティブペインが `Schema` / `Form` /（`Output Path`・`Filter`・`Output` のいずれか）のとき、中央帯をそのブロックだけ全幅にするトグル。タイトルに `[fullwidth]` を付与

---

## 操作要件

### グローバル操作

- `h j k l`, 矢印: 移動
- `0`, `$`, `gg`, `G`, `w`, `b`, `e`
- `i`, `a`, `o`, `O`, `v`, `Esc`
- `u`, `Ctrl-r`
- `?`: help overlay（表示中は `Esc` / `?` / ほぼ任意キーで閉じる。`reducer` の `ScreenMode::Help` 分岐に準拠）
- `q`: quit
- `r`: 出力保存

### フッター・ヘルプ（UI）

- フッター 1 行目: `APP` / `MODE` / `FOCUS` / `VALID` / `FIELDS` / `SCHEMA` / `FILTER`（幅に合わせ `truncate_spans`）
- フッター 2 行目: `KEYS` + 入力モード・アクティブペインに応じたショートカット（`key_cap_span`）
- ヘルプ本文は英語。フォーム見切れヒントも英語（`form_scroll_hint_line`）

### ペイン移動

- 数字キーで直接フォーカス
- `Tab` / `Shift-Tab` で循環
- マウスクリックでフォーカス

数字の意味は `standard` と `editor` で変わります。  
新しいキーバインドを足すときは、モード別の番号割当を壊さないでください。

### 削除系操作

現在の実装で基準とするのは以下です。

- `x` / `Delete`: 1 文字削除
- `dd`: 行削除
- `D` / `d$`: 行末まで削除
- `d0`: 行頭まで削除
- `dw`: 次単語境界まで削除

README にある全削除バリエーションを当然視しないでください。  
追加するなら `app/actions.rs` と `app/state.rs` の整合を先に確認してください。

### Form 専用操作

- `Enter`: single-line field 編集切替、または array button 実行
- `Tab` / `Shift-Tab`: insert 中の field / button 移動
- `h` / `l` / 左右矢印: `enum` / `boolean` / `oneOf` variant 行の値切替（`oneOf` は端で循環）
- `+`: array item 追加
- `-`: array item 削除
- `R`: schema default に戻す
- マウスクリックで field 選択や button 実行

### Schema Path / Output Path

- `Schema Path` は schema source を扱う
- `Output Path` は保存先 JSON パスを扱う
- `Schema Path` のみ補完サイクルを持つ
  - `Tab`: 次候補
  - `Shift-Tab`: 前候補
  - `Enter` / `Esc`: commit

### 保存

- `r` で保存
- 既存ファイルなら overwrite 確認を出す
- `y` / `Enter`: 上書き
- `n` / `Esc`: キャンセル

### ログ文言（フォーム）

- スカラー確定（`commit_form_field`）: `form field <dot.path>: <old> -> <new>`（`json_scalar_display_at_path` + 長さ上限）
- `oneOf` 枝変更: `form oneOf <path>: branch index <old> -> <new>`（初回は `old` を `default` と表示しうる）

---

## バリデーションとエラー表示

### バリデーション方針

- `jsonschema` crate で Draft 2020-12 を使う
- フィールド更新後に document を再検証する
- 保存前にも全体検証する
- invalid な document は保存しない

### エラー表示

- form の field error と log の両方で見えるようにする
- schema parse error は `Schema` ペイン下部と log に出す
- invalid schema 時は最後の正常な `schema_json` / `form` / `output_json` を維持する
- filter error は `Filter` ペイン下部と `Output` で識別可能にする
- 保存失敗、overwrite 確認、schema error は区別して扱う

エラーを黙殺しないでください。  
特に「無効な入力を受け取ったが state が silently 更新された」状態を作らないこと。

---

## JSON Schema 対応方針

### 実装済みとして扱ってよい範囲

- `type`
  - `object`
  - `array`
  - `string`
  - `number`
  - `integer`
  - `boolean`
  - `null`
- `properties`
- `required`
- `enum`
- `const`
- `default`
- `title`
- `description`
- `$defs`
- 内部 `$ref`
- `items`
- `prefixItems`
- `minItems`
- `maxItems`
- `pattern`
- `minLength`
- `maxLength`
- `minimum`
- `maximum`
- `exclusiveMinimum`
- `exclusiveMaximum`
- `format: "textarea"`

### 未対応または部分対応

- `oneOf`（部分対応: Form に variant 行を表示し `h`/`l` で循環切替。インスタンスが `const` 判別子と一致すれば推定枝を使用。切替でサブツリーは default 再生成）
- `anyOf`
- `allOf`
- `if` / `then` / `else`
- `dependentRequired`
- `dependentSchemas`
- `patternProperties`
- `additionalProperties` の編集 UI
- `contains` / `minContains` / `maxContains`
- 外部 `$ref`
- `type: ["string", "null"]` のような union type UI
- jq 完全互換

未対応キーワードは暗黙に無視してよいとは考えないでください。  
README や UI 上の期待値を上げる変更なら、少なくとも log や文書で制約を明示してください。

---

## Form 設計方針

### 基本原則

- DOM や Web フォーム前提の抽象化を持ち込まない
- `domain/form.rs` の `FormField` ベースの現在設計を尊重する
- scalar field の編集、array 編集、nested object 展開を壊さない

### 現在の field 表現

`FormField` は少なくとも以下を持ちます。

- `path`
- `key`
- `label`
- `description`
- `schema_type`
- `enum_options`
- `multiline`
- `required`
- `edit_buffer`
- `kind`（`Scalar` / `OneOfSelector` など）

フォーム拡張では、既存 field の責務を維持したまま最小の追加で表現できるかを先に検討してください。

### array 編集

現実装では以下を前提にしてください。

- 要素追加
- 要素削除
- object array を含む nested form 表示
- `minItems` / `maxItems` に基づく制御
- `prefixItems` を使う tuple 的初期展開

要素並び替えはまだありません。

---

## Filter 方針

`domain/filter.rs` の jq 風フィルターは最小サブセットです。

- `.`
- `.foo`
- `.foo.bar`
- `.array[0]`

エラー時は元 JSON を表示しつつ、error を別で持ちます。  
完全な jq パーサや外部ライブラリ導入を始める前に、この最小仕様を維持する目的がまだ十分かを確認してください。

---

## 出力要件

- `Output` は常に pretty JSON を表示する
- インデントは 4 spaces
- `Filter` は表示専用であり、元の `output_json` を破壊しない
- 保存先は `Output Path` から指定する

---

## アーキテクチャ原則

現在のディレクトリ構成:

```text
src/
  app/
    actions.rs
    reducer.rs
    state.rs
  domain/
    bundled.rs
    filter.rs
    form.rs
    validation.rs
  infra/
    clipboard.rs
    fs.rs
  ui/
    app.rs
  main.rs
```

### レイヤ責務

- `domain`
  - schema 由来の default 値生成
  - form field 展開
  - filter 評価
  - validation
- `app`
  - `AppState`
  - reducer
  - action 解釈
  - UI から見た状態遷移
- `infra`
  - file I/O
  - clipboard
- `ui`
  - ratatui 描画
  - マウス / キーボードイベント接続

### 守るべきこと

- UI に validation や schema 解釈を埋め込まない
- `AppState` だけが知るべき状態を widget 側に複製しない
- field 操作と pane 操作の責務を混ぜない
- 1 ファイルに責務を寄せすぎない

### 近い将来に分割候補となる責務

`ui/app.rs` と `app/state.rs` はすでに大きいです。  
以下は分割対象として常に意識してください。

- キーマップ解釈
- テキスト編集ユーティリティ
- マウス hit test
- pane レイアウト
- form 描画整形
- schema path 補完
- undo/redo 管理

ただし、大規模リファクタを先にやるのではなく、**変更対象の責務だけを切り出す**方針を優先してください。

---

## 状態管理方針

`AppState` はこのアプリの中心です。  
場当たり的な state 追加より、既存 state の意味を崩さないことを優先してください。

重要な state:

- `app_mode`
- `screen_mode`
- `input_mode`
- `active_pane`
- `schema_path`
- `schema_text`
- `schema_json`
- `output_json`
- `filter_text`
- `filter_outcome`
- `validation`
- `form_fields`
- `field_errors`
- `schema_error`
- `pane_cursors`
- `pane_histories`
- `visual_anchor`
- `logs`
- `overwrite_path`
- `form_button_focus`
- `one_of_choices`
- `form_mouse_scroll_accum`（`Form` ホイール感度。`Form` 以外にフォーカスが移ったらリセット）
- `schema_collapsed`
- `form_collapsed`

### 特に守る点

- `schema_text` と `schema_json` は分離する
- `filter_text` と `filter_outcome` は分離する
- 各ペインのカーソルは独立して保持する
- undo/redo は pane 単位の編集状態として扱う
- overlay 系 state は `screen_mode` で制御する

---

## テスト配置方針

現時点では `tests/` ディレクトリはなく、各モジュール内に unit test を置く構成です。  
変更時はまず既存スタイルに合わせて近いモジュールへテストを追加してください。

特に既存テストが厚い場所:

- `src/domain/form.rs`
- `src/app/state.rs`
- `src/app/actions.rs`
- `src/ui/app.rs`
- `src/infra/fs.rs`

新しい integration test を足すのはよいですが、まずは近傍のユニットテストで十分に押さえられるかを確認してください。

---

## Definition of Done

タスク完了条件:

- 期待する挙動が動く
- テストが追加または更新されている
- README や `AGENTS.md` と矛盾しない
- エラー表示とログが妥当
- 既存のキーバインドと pane 操作を壊していない

見た目だけ動く、またはテストだけ通る、のどちらか片方では不十分です。  
**挙動・設計・テスト・文書の整合**を揃えて完了としてください。

---

## 判断バイアス

迷ったら以下を優先してください。

- より単純
- より小さい変更
- より明示的
- より TUI ネイティブ
- よりテストしやすい
- より既存 README / 実装に整合する
- より silent failure を減らす
