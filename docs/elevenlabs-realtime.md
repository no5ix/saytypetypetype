# ElevenLabs 实时语音识别

## 使用

在“语音模型 → 在线模型”选择 **ElevenLabs · Scribe v2 Realtime**，展开参数并填写自己的 API Key，然后启用模型。语言留空为自动识别，可填写 `zh`、`en`、`pl` 等 2 或 3 位语言代码。

基础区提供 API Key、语言和背景语音过滤。“更多参数”提供语义顺滑、分段方式、服务端 VAD、附加语言和服务端热词；切换为手动分段后隐藏 VAD 调节，保留已保存数值。

- 服务端热词默认关闭；开启后发送现有启用词条，最多 50 条，每条最多 20 个字符，基础转写费用增加 20%。本地热词替换和 LLM 热词强化仍由原有设置控制。
- 语义顺滑对应 `no_verbatim`，去除口头填充、重复起句和不流畅表达。
- 附加语言用逗号分隔，保存为去重的语言代码数组。
- 自动分段只确认片段，不会停止录音。
- 音频固定为 16kHz、单声道 PCM16。首版不提供转写编辑、实体检测、时间戳或零保留设置。

## 配置

设置保存于 `audio.elevenlabs-realtime`。`api_key`、`language`、`enable_ddc` 复用现有字段；适配层映射到请求头 `xi-api-key`、`language_code`、`no_verbatim`。服务端 VAD 独立保存，不使用全局本地 VAD 参数。

API Key 沿用应用现有配置存储方式，不发送到 WebSocket URL，不输出到连接错误或日志。

## 停止与回放的边界

音频按顺序和实际时长发送；历史重试/重新识别也按此节奏回放，耗时约为录音长度加最终等待时间。停止后先排空发送队列，再发送最终 commit。

当前协议的 `committed_transcript` 没有 commit ID 或独立的结束确认，不能将停止后第一条确认消息认定为尾句。首版在最终提交后收集完整 10 秒窗口内的结果，再返回累计确认文本；若仍有临时文字，或收到有信号音频但没有任何确认事件，则报超时供用户重试。没有音频时直接结束。

**这意味着有音频的会话在停止后会额外等待约 10 秒。** 有界收集窗口仍不能证明服务端在窗口之外没有尚未返回的结果，真实服务的短句和尾字行为必须单独验收，不以模拟测试代替。

## 开发验证

```sh
pnpm exec vitest run
pnpm exec tsc --noEmit
pnpm run build:web
pnpm run lint:ci
cargo test --manifest-path src-tauri/Cargo.toml
```

Rust 测试使用本地回环 WebSocket，不需要 ElevenLabs Key，也不会产生 API 费用。

## 本次验证记录（2026-10-15）

- 前端：16 个测试文件、153 项测试通过；TypeScript 检查和生产构建通过。
- Rust：336 项测试通过，1 项原有迁移测试忽略；格式检查与 Clippy 通过。
- 浏览器预览：检查了参数布局、手动分段隐藏 VAD、附加语言去重。预览使用真实组件与内存配置，不连接 Tauri 或真实识别服务。
- 真实 ElevenLabs API、macOS/Windows 麦克风与自动粘贴尚未实测；合并前按下列清单验证。

## 用户验收清单

- [ ] API Key 保存、重启、恢复默认以及切换回豆包/离线模型正常。
- [ ] 不足 2 秒短句、说完立即停止：无尾字遗漏。
- [ ] 中英文及混合讲话：分段文字不重复，英文单词不粘连。
- [ ] 超过 40 秒讲话、多次自动分段：前文完整，录音不提前结束。
- [ ] 纯静音、长停顿：能够结束，连接行为合理。
- [ ] 背景语音过滤、语义顺滑：开关效果符合预期。
- [ ] 热词默认关闭；额外付费测试单独启用并记录。
- [ ] 错误 Key、余额不足、断网重连：提示清楚，无无限重试。
- [ ] ESC 后不粘贴；历史重试与重新识别正常。
- [ ] macOS 真机完成；Windows 真机单独完成。

交付前的自动检查和参数界面预览不代表上述真实服务及系统集成场景已经通过。用户明确验证通过前不合并 master。

协议依据：[Realtime API](https://elevenlabs.io/docs/api-reference/speech-to-text/v-1-speech-to-text-realtime)、[官方 Python SDK](https://github.com/elevenlabs/elevenlabs-python/tree/main/src/elevenlabs/realtime)。
