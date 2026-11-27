# C-018-STREAM-CORE 只读就绪审查

> 审查时间：2026-11-25
> 状态：卡片 `pending`，被 `C-018-HTTP-SAFE` 与 `C-018-ROUTER-COST` 的 delivered 状态阻塞；本文不构成派卡或实施授权。

## 1. 当前代码事实

- [stream_util.rs](<src/providers/stream_util.rs>) 仍是无上限 `Vec<u8>`，完整行与 EOF tail 都使用 `from_utf8_lossy`。
- [stream_state.rs](<src/providers/stream_state.rs>) 的 `UnifyTerminal`：
  - `harvest` 仍不保存 `thinking`，terminal 固定 `thinking: None`；
  - Err 后虽然把 phase 设为 `Errored`，但下一次 poll 仍先 poll inner；inner Pending 时会继续等待，未实现“立即关闭”；
  - 非错误 EOF 仍合成 done；这是有意的安全网，但必须与卡片的“模糊 EOF 显式错误口径”裁定统一。
- 四个 Provider 都已使用 `bytes_stream()`，但都没有在建立流前校验 `Content-Type`。
- `src/providers/sse/` 尚不存在。
- Ollama 生产流通过 `line_stream`；测试夹具中还有两处 `from_utf8_lossy`，不属于生产解码路径，移除时应避免误删合法 fixture。

## 2. 行数与台账风险

| 文件 | 当前行数 | threshold | 风险 |
|---|---:|---:|---|
| `src/providers/stream_util.rs` | 128 | 500 | 低 |
| `src/providers/stream_state.rs` | 440 | 500 | **高，容易跨阈值** |
| `src/providers/mod.rs` | 427 | 500 | 中，新增模块声明/错误映射可能跨阈值 |
| `src/providers/compat.rs` | 2129 | 500 | 已在台账 |
| `src/providers/anthropic.rs` | 1688 | 500 | 已在台账 |
| `src/providers/google.rs` | 1812 | 500 | 已在台账 |
| `src/providers/ollama.rs` | 860 | 500 | 已在台账 |

开发必须使用卡片 c5 的 `newly-oversized old=0` 分支，不能假设 `batch_base` 一定存在 `stream_state.rs` 或 `mod.rs` 的 baseline。

## 3. 实施前必须钉住的设计点

1. **公开 API 兼容**：保留 `line_stream` 现有签名；严格有界解析器作为内部新模块，旧 API 委托到严格实现或明确标注兼容行为。
2. **SSE 事件语义**：支持 `data:` / `data: `、多行 data、CRLF、CR-only、BOM、注释、`event:error`；按空行 dispatch，不把多行 data 拆成多个 JSON。
3. **Content-Type 策略**：
   - OpenAI-compatible / Anthropic / Gemini：接受 `text/event-stream`（可带 charset）；
   - Ollama：接受 `application/x-ndjson` / `application/json`；
   - 必须明确缺失 Content-Type 时是拒绝还是按协议宽容，不能静默空成功；建议默认拒绝并在测试中钉住。
4. **UTF-8**：完整行/EOF tail 用严格 `String::from_utf8`；非法字节返回 Parse/Stream，不能替换 U+FFFD。
5. **资源上限**：行、事件、累计工具参数都要有上限；超限立即 Err 并关闭，不能继续 poll。
6. **terminal thinking**：保存首个 terminal 的 thinking，或拆成非 terminal reasoning chunk；`thinking_done` 至多一次。
7. **Err 后关闭**：在 `poll_next` 循环顶部检查 `Errored`，直接 `Ready(None)`，不能先 poll 一个可能 Pending 的 inner。
8. **EOF 口径**：卡片裁定 clean EOF 可 flush 完整工具且 finish_reason=None；transport/parse error 绝不 flush。需要测试证明“完整工具”与“半截工具”可区分。
9. **代理 raw stream**：B-AUD-3 已让代理走 `stream_raw_with`，不会经过 `UnifyTerminal`；本卡改 terminal 语义时不得破坏代理自己的 truncation 状态机。

## 4. 测试与门禁注意

- c1 要求恰好 7 条 `audit_2026_stream_*`，必须走真实 provider 字节解析或 `LmrsClient::stream_with`，不能只测测试内重写逻辑。
- c3 注入后必须验证恢复文件 SHA 等于 h0；Windows 下 Cargo 可能复用注入态二进制，需要强制刷新 mtime 或清理对应 artifact 后复跑。
- c6 的 `should_panic` 基线行号已因前序提交漂移；按“文件 + 所属测试函数 + 属性文本”的稳定位置集合比对，不能锚死旧行号。
- 所有 Rust 文件保持 LF，`tests/hotspot_ledger.json` 保持 CRLF；普通 `git diff --check` 对 CRLF 会误报，需同时跑 `-c core.whitespace=cr-at-eol diff --check`。
- semver 使用 `--baseline-version 0.1.6 --default-features` 与 `--all-features` 双口径。

## 5. 结论

卡片范围与当前热点重叠匹配，无静态 scope 缺口；可以按现有卡实施。最高风险是 `stream_state.rs` 跨阈值、Content-Type 宽容策略未定义，以及 Err 后 Pending 关闭语义。建议开发先落“有界严格解析 + Err 立即关闭”的最小编译闭环，再补 7 条生产路径测试。
