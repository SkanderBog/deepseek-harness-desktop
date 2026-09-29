---
name: hatch-dsh-pet
description: Hatch a custom desktop pet for the dsh-tauri-pet plugin by authoring a pet folder (pet.json plus an 8-column sprite atlas) under the DSH data directory, then validating it against the host contract so it shows up in Settings → 宠物. Use when the user asks to create, hatch, import, replace, or repair a custom pet, when they want the pet to react to conversation activity, or when a pet folder they added does not appear in the pet list.
metadata:
  author: Hairyf
  version: "2026.09.29"
---

# Hatch DSH Pet

把一只自定义宠物养进 `dsh-tauri-pet`：在 DSH 数据目录下产出一个宠物文件夹，宿主下次枚举宠物列表时会自动识别，选中后桌宠窗口即可加载。

## 何时使用

- 用户说「养一只宠物」「创建宠物」「做一个自定义桌宠」「换成我自己的形象」。
- 用户已经把素材放进 `pets/` 目录，但设置页的宠物列表里看不到它。
- 用户想替换、修复或重新烘焙一只已有自定义宠物。
- 用户的宠物在列表里可见但选中后桌宠窗口不显示（通常是图集尺寸或格式不合约）。

不要用于：Codex 宠物的 `.zip` 导入（那走设置页 Codex 标签页的「导入」按钮，落点在 `~/.codex/pets`），也不要用于修改内置预设宠物（它们在 `src-tauri/resources/manifest.jsonc` 里，由主程序提供）。

## 落点

DSH 数据目录按以下顺序确定，先取用户环境里已有的值再动手：

1. 非空环境变量 `DSH_HOME`（主程序为内核注入的就是它）。
2. 否则 `~/.dsh`（发行版默认）。
3. 调试构建固定用 `~/.dsh.dev`，不看 `DSH_HOME`。

宠物根目录 = `<DSH_HOME>/pets`，每只宠物一个**直接子目录**：`<DSH_HOME>/pets/<petId>/`。宿主只枚举直接子目录，不递归、不跟随符号链接，所以不要套一层 `my-pet/pet/`；目录名与 `pet.json` 里的 `id` 不必一致，`id` 才是标识。

`<petId>` 规则：1~64 个字符，只允许 ASCII 字母、数字、`-`、`_`。列表里展示的 id 是 `chat:<petId>`。

宠物文件夹至少两个文件：

```
<DSH_HOME>/pets/<petId>/
├── pet.json
└── spritesheet.webp        # 文件名任意，PNG 或 WebP 都行
```

## pet.json

camelCase 字段，只有 `id` 与 `spritesheetPath` 必填；`pet.json` 本身不得超过 64 KiB。

```json
{
  "id": "my-pet",
  "displayName": "My Pet",
  "description": "在会话里陪我的桌宠",
  "spriteVersionNumber": 2,
  "spritesheetPath": "spritesheet.webp"
}
```

- `displayName` 省略时列表回落到 `id`；`description` 可选。
- `spriteVersionNumber` 只能是 `1` 或 `2`，其他值会让这只宠物被整个跳过。`2` 表示 11 行图集（含两个朝向行），`1` 表示 9 行图集。**新宠物一律写 `2`。**
- `spritesheetPath` 必须是相对路径、使用正斜杠，且不含 `..`、不绝对、不含 `\`、不含 `:`。

## 图集契约

宿主按固定网格切帧，**不看像素内容，只校验头部尺寸**：

- 格式：PNG 或 WebP，透明背景。
- 列数固定 8；行数 9（v1）或 11（v2）。
- 宽必须是 8 的倍数；高必须是 9 的倍数或 11 的倍数。
- 单边不超过 16384 像素，总像素不超过 64 Mi；图集文件不超过 8 MiB。
- 推荐规格：8 列 × 11 行，每格 192×208，整图 **1536×2288**，`spriteVersionNumber: 2`。

行序、每行动作帧数与逐帧时长见 `references/atlas-layout.md`，导出素材时按那张表对齐。行的用途由宿主固定：第 0 行是待机，第 1/2 行是移动，其后各行分别对应挥手、跳跃、失败、等待、运行、复查，最后两行是按角度采样朝向的 look 行。

标准动作行只用到左侧若干格，**末帧之后的格子必须完全透明**，否则会露出脏像素。look 行则要填满 8 格。

## 步骤

1. 确定数据目录与宠物目录，检查 `<DSH_HOME>/pets/<petId>/` 是否已存在（已存在就先和用户确认是覆盖还是换 id）。
2. 拿到素材。DSH 不自带图像生成工具，按可用能力挑一条路：
   - 会话里有生图能力，就让模型按上面的网格与行序产出图集；
   - 用户提供单帧或现成图集，就检查并归位；
   - 都没有时，向用户索取素材，不要自行编造像素内容后声称已完成。
   合成图集可以用 Node 内置模块（`node:zlib` 足以手写 PNG，`node:fs` 足以拼接 WebP 分块）；**不要依赖 Python、PIL、sharp、ImageMagick，宿主和内核都不保证它们在场。**
3. 写好 `pet.json`，把图集放进同一个目录。
4. 用本技能自带的校验脚本核对契约：

   ```bash
   node "<skill 基目录>/references/validate-pet.mjs" "<DSH_HOME>/pets/<petId>"
   ```

   脚本会按宿主同一套规则报错：id 非法、路径不可移植、清单超限、格式不可解析、宽高不整除、尺寸超限、声明版本与尺寸推定不一致。全部通过时打印解析出的网格（版本 / 列 / 行）。
5. 让用户确认：打开 设置 → 宠物，新宠物出现在 Pets 列表里；点「选择」即可启用。列表是在设置页挂载时重新枚举的，无需重启主程序。

## 失效时的表现

- **完全不在列表里**：`pet.json` 读不出、id 非法、或目录不是 `pets/` 的直接子目录。宿主会静默跳过读不出清单的目录，不报错。
- **在列表里但缩略图是占位方块**：图集读不出或格式/尺寸不合约，列表项仍会保留，缩略图生成失败。
- **能选中但桌宠窗口不显示**：`get_pet_asset` 校验失败的典型症状，多半是宽不是 8 的倍数，或高既不是 9 的倍数也不是 11 的倍数。行数由**图集高度**推定，不看你写的 `spriteVersionNumber`：1536×2288 只会解析成 11 行，声明成 `1` 不会让它变成 9 行图集，只会被校验脚本记为一条 note。
- **选择后状态回落到预设宠物**：写入的 active id 不是合法限定 id；自定义宠物必须是 `chat:<petId>`。

## 边界

- 不要动 `~/.codex/pets`，那是 Codex 宠物的落点，由导入流程管理。
- 不要在宠物目录里放符号链接；宿主拒绝跟随。
- 不要为了「让它显示」去改宿主的 Rust 代码或 patch `src-tauri`：契约已经支持任意数量的自定义宠物目录。
- 交付前必须跑校验脚本，并把真实输出贴给用户；不要凭肉眼断言尺寸。
