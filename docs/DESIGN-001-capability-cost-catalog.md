# llmrust · 能力表 + Rust 类型草图（设计稿 v1）

> 定位：这是 llmrust 0.1.4 的**地基设计稿**——它不写业务逻辑，只回答一件事：
> **"每家厂商、每个模型，到底支持什么、要多少钱"这张表，在 Rust 里该长什么样。**
> 上游对应格：`0.1.4 / N1 Capability Truth`（能力真相）。
> 本稿只出设计，不动一行 llmrust 源码。

---

## 〇、缘起（按时间顺序，含守溥原话）——**先看清这件事是为了什么**

> 这一节放在最前面：**厂商覆盖是第二件事，第一件事是"让主项目把那笔钱省下来"。**

### 0-1 起点：主项目的 **AC-16 成本缺口**（不是"想多接几家厂商"）

- 主项目 `agent-core` 缺陷 **AC-16（P0，成本）**：**每轮把整段历史全量重发给模型**，
  静态可推导 → 50 轮会话重复付费约 **25×**（`agent/request.rs:18-23` 每轮 `self.messages.clone()`）。
- 我方判定（`RULES §〇-A D-06`）：**"打缓存断点"是能耐 → 归 `yesagent-ai`**，不进 core；
  core 只负责**前缀稳定**（K10，已落地）。
- 于是省钱的链路是：**前缀稳定（core，已做）→ 缓存断点（yesagent-ai）→ 而 `yesagent-ai` 的能力来自 `llmrust`**。

### 0-2 守溥指定的参照：先看 maka 怎么做

- 守溥原话：**"省钱那个我不是说让你去参考 maka 的方式吗？这是老生常谈了……你先去看看 maka 怎么做的"**
  → 读 maka《Compaction Is a Projection》（日志只增不改 / 检查点带覆盖声明 / 滚动更新 / 宁可少看也不看假历史）。
- 守溥原话：**"我们来把压缩算法和 kv 缓存之间的事情来好好说一遍，这个也关乎到我们整个项目的问题"**
  → 结论：**缓存命中只认"逐字节前缀相同"**；**"重发"不贵，"前缀变了"才贵**；
  压缩只能动头部、动完冻结；检索/图谱只能往尾巴加料。

### 0-3 卡点浮出：`llmrust` 能力不足（**这就是本稿的由来**）

- 守溥问：**"你说的这些里面也包含 llmrust 的能力不足的问题吗？"**
  → 实测：llmrust **有电表、没开关**——能读到 `cache_read_input_tokens` / `cache_creation_input_tokens`（计量），
  但 `ChatRequest` 无缓存字段、`ContentPart` 只有 Text/ImageUrl、Anthropic `build_body` 不引用 `extra`，
  **发不出 `cache_control`**（`本地 llmrust 检出（0.1.3）`，pin `=0.1.3`）。
- 守溥更正：**"llmrust 是我们自己的开源项目不是别人的，他也是为了做这个项目我独立出来的"**
  → 因此这不是"求上游"，而是**我们自己的一个工作项**（另一个仓、自己的发版）。

### 0-4 **原点令（守溥原话，逐字）**

> **"首先 llmrust 是为我们主项目服务的，那么他首先要支持的就是为我们主项目服务的事情，
> 现在他能力不足 1.4 就必须更改里面的里程碑必须来支持我们的项目明白吗？"**

**读法（我的落实）**：
1. llmrust 的**第一职责 = 支撑主项目**，不是它自成一派；
2. 1.4 的**里程碑按主项目需求倒排**（它现有六格 `N0 Guard Integrity / N1 Capability Truth / N2 Silent Failure /
   N3 Evidence Depth / N4 Governance Scale / N5 Release` 里**没有"能力/成本"格** → 需并入或新开）；
3. **不许排过多需求**（守溥后续原话："1.4 的内容主要就是为了我们主项目服务，不要排过多的需求进去"）。

### 0-5 随后才加的第二件：厂商覆盖（守溥原话）

> **"智谱，DeepSeek，mimo，opencode，commandcode 这些都要，Claude，gpt，qwen 这些这些都要官方支持明白吗？"**

→ 因此本稿的**优先级**是：
**① 让主项目能拿到缓存价（成本）→ ② 让客户要接的模型能接进来（覆盖）→ ③ 顺带把"能力真相"变成机器可验的物（治漂）。**

---

## 一、目的与边界

**要解决的病**（都是实测出来的，不是设想）：

1. 上游现在"能力"是靠**运行时字符串**表达的——`LlmError::Unsupported { feature: String, message: String }`，
   provider 默认实现返回它。**TypeScript/Python 只能这么干，Rust 不该这么干。**
2. 上游**没有机读的能力/定价表**：`docs/CAPABILITIES.md` 当时是 **164 行**散文
   （**本稿写作时快照**；**2026-09-11 复测 = 209 行**）。**该缺口已被 `CAP-006`/`CAP-007` 部分关闭**：
   现文件的能力矩阵**由 `llmrust.models.json` 生成并受门禁保护**（人工改一字即红），
   另新增"验证层级（SPCC §6.2）"一节。**逐段归属未测量，故不声称各行来自哪张卡**。全仓没有
   `context_window` / `max_output` 一类结构化表（仅 3 处零散命中）。
3. 上游**定价只有两个价**：`ModelPricing { prompt_per_1k, completion_per_1k }`，
   且**它自己的注释写着**"不覆盖 provider-specific discounts、cached-token rates、rounding"。
   → 缓存断点就算发出去了，也算不出省了多少钱。
4. 结果：能力说错**没人能机器发现**；厂商差异散在代码分支里（我们自己主项目就吃过同一课）。

**边界（不做什么）**：

- 不替调用方决定"打几个断点、打在哪"（那是上层策略）；
- 不引入任意 JSON registry（上游已明令禁止的方向）；
- 不把 LiteLLM 的"运行时字典 + 查表"形态搬进来（见 §六 禁止令）。

---

## 二、表该有哪些列（第一阶段 ↔ 可后补）

| # | 列 | 类型 | 阶段 | 用途 / 来源 |
|---|---|---|---|---|
| 1 | `id` | `ModelId`（newtype） | **必做** | 模型标识（**不与 provider 名互串**） |
| 2 | `provider` | `ProviderSlug`（newtype） | **必做** | 厂商标识 |
| 3 | `context_window` | `u32` | **必做** | 容量推导（主项目"reserve=窗口/4 上限 16k"要用） |
| 4 | `max_output` | `u32` | **必做** | 请求上限校验 |
| 5 | `prompt_price` | `Money`（newtype, per-1k） | **必做** | 计费 |
| 6 | `completion_price` | `Money` | **必做** | 计费 |
| 7 | `cache_read_price` | `Money` | **必做** | **缓存命中价（≈0.1×）——现在没有** |
| 8 | `cache_write_price_5m` | `Money` | **必做** | **写缓存价 5 分钟档（≈1.25×）——现在没有** |
| 9 | `cache_write_price_1h` | `Option<Money>` | 可后补 | 1 小时档（≈2×），`None` = 不支持该档 |
| 10 | `cache_spec` | `Option<CacheSpec>` | **必做** | 缓存能力本体（见 §三），`None` = 不支持缓存 |
| 11 | `reasoning` | `Option<ReasoningSpec>` | **必做** | 是否支持 thinking / 档位映射 |
| 12 | `tool_calling` | `ToolCalling`（enum） | **必做** | 支持/不支持/仅流式等 |
| 13 | `embeddings` | `bool` | 可后补 | 上游已有 embeddings 默认 Unsupported |
| 14 | `stream_format` | `StreamFormat`（enum） | **必做（与需求③同批）** | 各家流式差异；**它是需求③"真 HTTP 元数据"的硬落点，不许后补**（监工 复核补 1：原标"可后补"与 §五 自相矛盾，若后补则需求③落不了地） |
| 15 | `notes` | `&'static str` | 可后补 | 已知怪癖（**由表生成，不再手写散文**） |

**列清单的取舍原则**：**只用得上、且能机器判的才进表**。像"推荐温度"这类审美字段不进。

---

## 三、Rust 类型草图（不是照抄 TS/Python）

### 3-1 newtype：防串账

```rust
pub struct ModelId(&'static str);
pub struct ProviderSlug(&'static str);
pub struct Money(f64);          // 每 1k token 的美元价
```

> 为什么：主项目 D-18 已用同一思路（`CheckpointId`/`SessionId`/`TaskId` 各自独立类型）。
> 把 `ModelId` 与 `ProviderSlug` 混用是这类库最常见的低级错。

### 3-2 能力 = 类型，不是字典字段（**审核官 F-2 打回后已改：trait 只留"有没有"，数字全进数据**）

```rust
/// 缓存能力：**实现了才存在**；没实现 = 类型上没有，调用方根本调不到。
/// 注意：这里**只放"协议级"常量**（Anthropic 全系都是 4 个断点）。
/// **凡逐模型不同的数字（最小可缓存长度等）一律不进关联常量**，见下方 CacheSpec。
pub trait Caching {
    /// 最多几个断点（Anthropic 协议级 = 4；LiteLLM 也是这么当"协议常量"用的）
    const MAX_BREAKPOINTS: usize;
}

/// 逐模型的能力数据（**挂模型、不挂厂商**）——因为同一厂商不同模型不同：
/// 实测 Anthropic Sonnet/Opus = 1024，**Haiku = 4096**；阶跃 256；通义 1024。
/// 若把 MIN_CACHEABLE_TOKENS 写成 trait 关联常量，就表达不了 Haiku 的例外（原稿之错）。
pub struct CacheSpec {
    pub mode: CacheMode,                 // Auto | Explicit | Both | None
    pub min_cacheable_tokens: u32,       // ← 逐模型数据
    pub ttl: Ttl,                        // ← 与淘汰正交（审核官 F-4）
    pub eviction: Eviction,
    pub supports_tools_cache: bool,      // 工具定义上能否打断点
}
```

### 3-3 策略 = enum（不是字符串）

```rust
pub enum CacheRetention { None, Short, Long }   // 照 pi 的三档，但用 enum 钉死取值域
```

**断点位置是库内约定、不是调用方的活**（照 pi）：
> 同一个 `ephemeral` 标记贴到 **system 末尾 / 最后一个工具定义 / 最后一条消息的内容块**。

### 3-4 断点数量进类型（越界早失败）

```rust
/// 断点数量写进类型；越界在构造期就死（无需运行时检查）
pub struct Breakpoints<const N: usize>([BlockRef; N]);
```

### 3-5 "不支持"从运行时字符串升级为类型态

```rust
pub enum Capability { Caching, Reasoning, ToolCalling, Embeddings }

pub enum CapabilityError {
    NotSupported { capability: Capability, provider: ProviderSlug },
}
```

> 现状 `LlmError::Unsupported { feature: String, .. }` 保留兼容，但**新增能力一律走 enum**。

### 3-6 目录在构建期落地（运行时零解析）——**⚠ 本小节为初稿形状，已被 §18-3 取代**

> **审核官 F-2 打回**：下面这个"扁平 `ModelSpec`"（把 `provider` 与模型混在一行、没有 `protocol/base_url/key_env`）
> 已被 §12-4 与 §18-3 的**两维模型**（`ModelSpec` + `AccessPath`）推翻。
> **本小节只保留"构建期落地 + `deny_unknown_fields`"这一条机制**；**字段定义以 §18-3 为准**。

```rust
/// 机制（保留）：表在构建期落地 → 运行时零解析、零字典查找
pub const MODEL_CATALOG: &[ModelSpec] = include!("model_catalog.generated.rs");
pub const ACCESS_PATHS:  &[AccessPath] = include!("access_paths.generated.rs");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]        // 表里写错字段 → 解析期报错，不静默失效
pub struct ModelSpec { /* ← 字段见 §18-3，不在此重复 */ }
```

价格字段（**已按审核官第三条统一为 Option**）：

```rust
pub struct PriceTiers {          // 支持"超长上下文分档"（约束⑥）
    pub prompt_price: Option<Money>,
    pub completion_price: Option<Money>,
    pub cache_read_price: Option<Money>,
    pub cache_write_price_5m: Option<Money>,   // ← Option（多数家写不加价）
    pub cache_write_price_1h: Option<Money>,   // ← 仅 Anthropic 系上报
}
```

### 3-7 表内一致性用 const 断言（编译期）

```rust
// 例：声明支持缓存读价的模型，必须同时给 cache_spec；否则编译就报
const fn assert_cache_price_consistent(s: &ModelSpec) { /* ... */ }
```

---

## 四、生成管线（数据 → 类型 → 门）

```
① 源数据（机读，可对齐 models.dev / LiteLLM 表）
        ↓  xtask 生成器（构建期或显式命令）
② model_catalog.generated.rs   ← 强类型常量，进二进制
        ↓  check-model-data 门（照 pi 的 check-model-data.ts）
③ CI 级校验：字段齐全 / 价格非负 / 能力与价格自洽 / 目录 ↔ CAPABILITIES.md 一致
```

- **`docs/CAPABILITIES.md` 改成"由表生成的视图"**（不手写）——这一条直接治它 N1 那格"能力真相"。
- 生成器**不联网**（表是仓库里的数据文件），保证构建可复现（P8/P13 同款纪律）。

---

## 五、三件需求怎么挂在这张表上

| 需求 | 挂在表的哪里 |
|---|---|
| **缓存断点发送**（卡 1） | `cache_spec` + `CacheRetention` + 库内约定打点 + §3-4 断点类型 |
| **厂商覆盖补齐**（卡 2） | 表里新增 4 家的 `ModelSpec` 行（Qwen / Zhipu / Grok / StepFun）+ provider 适配 |
| **真 HTTP 元数据**（卡 3） | 表加 `stream_format`；错误侧用 `CapabilityError` 类 enum 携带真实 status |
| **成本能算对账** | `cache_read_price` / `cache_write_price_*` → `estimate_cost` 按档计价 |

---

## 六、禁止令（防止把别人的短板搬进来）

**禁止**在 llmrust 出现以下三件套：

1. **字典查表**（运行时字符串查能力/价格）；
2. **运行时 `if` 分支堆**（`if provider == "anthropic" { ... }`）；
3. **布尔旗标堆**（`supportsA: bool, supportsB: bool, ...` 组合爆炸、可互相矛盾）。

> 正确形态：**能力用 trait/关联常量声明，策略用 enum，数据构建期落地，越界用类型挡。**
>
> **正面表述（§十七 展开）**：LiteLLM 自己的 Rust 版把这件事说成——
> *"**The test for a good abstraction is that adding the next provider is a few declarative lines, not a new file of duplicated flow.**"*
> 我们照此立标准：**新增一家厂商 = 目录里加若干行数据 + 一个 `const` 配置；不许新增一套并行流程。**

---

## 七、DoD（机器验）

**0. 默认模型先定案（监工复核补 2 · 分水岭）**：必须给出主项目默认模型 `stepfun/step-3.7-flash`
的 `cache_spec` 与三档价格的**明确结论（支持 / 不支持 / 未知）**；**未知须先查厂商官方文档，
不许默认填"支持"**。理由：若它是"不支持"，AC-16 打点止血在**默认配置下直接归零**，我们会在一轮空干。
> **已定案：支持（Auto / 0.2× / 256 min / LRU），见 §二十**。若后续发现它"不支持"，本条立即回升为首要风险。

1. `MODEL_CATALOG` 覆盖**至少**：现有 **7** 家（以 `docs/CAPABILITIES.md` Provider support matrix 为准：
   OpenAI / DeepSeek / Moonshot / OpenRouter / Anthropic / Gemini / Ollama）＋ 新增 4 家（Qwen / Zhipu / Grok / StepFun）的常用模型；
   > **勘误（2026-09-10，审核官 S-2 指出、我方复核确认）**：本节原写"现有 9 家"**来源不明**——
   > 实测矩阵是 **7 家**；"9"疑为把 `src/providers/` 下的基建文件（`http`/`retry`/`stream_state`/`stream_util`/`mod`）也计入。按 P8"数字要么可复现要么别写"，已改为 7 并附复核来源。
2. 每个声明 `cache_spec.is_some()` 的模型，**必须**同时给 `cache_read_price` 与 `cache_write_price_5m`（const 断言 + 门）；
3. `docs/CAPABILITIES.md` 与目录**逐行一致**（由生成器产出，人工改动即红）；
4. 目录数据**字段齐全、价格非负**（数据门）；
5. 既有 provider 行为零回归（全门绿：fmt / clippy / 既有测试）；家数见第 1 条（**7**，非 9）；
6. `estimate_cost` 新增缓存价路径，并有单测：**命中读价 = 0.1×、5m 写价 = 1.25×、1h 写价 = 2×** 三种情形的算例；
   （价格以 §十四 的**逐家官方值**为准，上列倍率只是 Anthropic 的示例。）
7. **编译失败测试（Rust 独有，必须做）**：用 `trybuild` 固化三条"编译期就该失败"的反例——
   ① 对不支持缓存的载体调 `with_cache_retention`；② 断点数超过该厂商 `MAX_BREAKPOINTS`；
   ③ 价格表声明了缓存读价却没有 `cache_spec`。
   → 这三条在 LiteLLM / pi / maka 里**只能运行时发现**，在这里是**机器可验的编译期约束**（§十七 的正面对应）。
8. **数据门**：生成器与 CI 各跑一遍"目录自检"（字段齐全 / 价格非负 / 能力与价格自洽 / `deny_unknown_fields` 解析通过）。

---

## 八、与上游里程碑的对应

| 本稿内容 | 归属格 |
|---|---|
| 能力表 + 生成管线 + CAPABILITIES 自动视图 | `0.1.4 / N1 Capability Truth` |
| 数据门 / const 断言 / 目录一致性检查 | `0.1.4 / N0 Guard Integrity` |
| 缓存断点发送（形态照 pi） | 挂 N1（或新增"成本能力"格，由上游定） |
| 定价补缓存价 | 挂 N1（钱属于"能力真相"的一部分） |

---

## 九、待定（需要上游/守溥拍）

1. **源数据从哪来**：自建（各家官方文档人工维护）还是对齐 `models.dev` / LiteLLM 表？
   → **已定：自建为主、业界表只当校对尺**（监工复核采纳）。
2. **表落点**：`src/model_catalog/`（数据文件 + 生成物分目录）还是单文件？
   → **已定：`src/model_catalog/`**（数据与生成物分目录）。
3. **是否新增一格**：缓存能力与定价要不要单独立 `N6 Cost Capability`，还是并入 N1？
   → **已定：并入 N1，不新开格**。

---

## 十一、四家目标厂商的实况数据（来自行业标准表，LiteLLM `model_prices_and_context_window.json` @ 2026-09-10）

> 取数方式：`gh api repos/BerriAI/litellm/contents/model_prices_and_context_window.json -H 'Accept: application/vnd.github.raw'`
> → 落盘 2.25 MB / 3886 条目 → `ConvertFrom-Json -AsHashtable` 解析（PS 因"大小写重名键"必须加 `-AsHashtable`）。

| 模型（厂商）＋**接入路径**（表内键） | 上下文 | 最大输出 | 输入价/M | 输出价/M | **缓存读价/M** | 读价倍率 | `supports_prompt_caching` |
|---|---|---|---|---|---|---|---|
| xAI Grok-3（`xai/grok-3`，**xAI 官方路径**） | 131,072 | 131,072 | $1.25 | $2.50 | **$0.20** | **0.16×** | True |
| **DeepSeek V4 Flash**（`dashscope/deepseek-v4-flash`，**经阿里百炼**接入） | **1,000,000** | 393,216 | $0.20 | $0.40 | **$0.04** | **0.20×** | True |
| **智谱 GLM-5.1**（`dashscope/glm-5.1`，**经阿里百炼**接入） | 202,745 | 131,072 | $1.40 | $4.40 | **$0.26** | **0.186×** | True |
| **智谱 GLM-5.1**（`zai/glm-5.1`，**智谱官方路径**） | 202,745 | 131,072 | $1.40 | $4.40 | **$0.26** | 0.186× | True |
| 阶跃 StepFun 3.7-flash（`novita/stepfun/step-3.7-flash`，第三方转售） | **262,144** | 256,000 | $0.20 | $1.15 | **$0.04** | **0.20×** | True |
| 阶跃 StepFun 3.7-flash（`deepinfra/stepfun-ai/Step-3.7-Flash`，第三方转售） | 262,144 | — | $0.20 | $1.15 | **$0.04** | 0.20× | True |

> **⚠ 表键前缀 = "接入路径"，不是"模型厂商"**（守溥 2026-09-10 指正，已核实）：
> 实测 `dashscope/` 前缀下 **45 条**里，**不属于阿里的有** `deepseek` 3 条、`glm` 2 条、`kimi` 1 条
> —— **`dashscope` 是阿里百炼这个"接入平台"，上面挂着别家的模型**。
> 同一个 `glm-5.1` 在表里**有两条路径**（`dashscope/glm-5.1` 与 `zai/glm-5.1`），`litellm_provider` 分别是
> `dashscope` 与 `zai`，**价格相同**。
> ⇒ 见 §18**「模型」与「接入路径」是两个正交维度**，目录必须分开建模。

**覆盖缺口（表里没有的）**：
- `zhipu/` 键 = **0 条** → 智谱**自有 API 不在 LiteLLM 表里**（GLM 模型是以 `azure_ai/FW-GLM-*`、`dashscope/glm-*`、`volcengine` 等**转售/托管行**出现的，共 128 条）；
- `stepfun/` 自有 API 键 = **0 条**（只有上面两条**转售行**）；
- ⇒ **智谱与阶跃的自有 API 行，只能我们自建**（与 §九 第 1 条"自建为主"一致），业界表**只用于交叉校对**。

### 由实况数据得出的两条口径更正（原稿假设被数据推翻）

1. **缓存读价不是行业统一的 0.1×，而是逐家不同**：Anthropic = 0.1×；**Grok ≈ 0.16×**；**dashscope 系 / StepFun ≈ 0.20×**。
   → 因此表中 `cache_read_price` **必须是每模型的绝对值**（或显式倍率），**禁止**全局硬编码 0.1×。
2. **写缓存价（`cache_creation_input_token_cost`）在这些家普遍为空** → 很多厂商**写缓存不加价**（写价 = 普通输入价）。
   → 因此 `cache_write_price_5m/1h` 保持 **`Option`**，**禁止**默认填 1.25×/2×（那是 Anthropic 的口径，不是普适）。

### 仍缺（不许猜）

| # | 待查 | 归谁 |
|---|---|---|
| A | 智谱 / 阶跃**自有 API** 的上下文窗口与三档价格（表里只有转售行） | 自建（官方文档） |
| B | StepFun 缓存是**自动**还是**需标记**（官方文档《Prompt 缓存最佳实践》在 `platform.stepfun.com`，本执行环境 DNS 不可达） | 需能直连该站的人补 |
| C | 各家的**最小可缓存长度**与 **TTL 档位** | 自建（官方文档） |
| D | 上述四家的 `cache_write_price_*` 是否真的为 0（写不加价）还是**表里未收录** | 自建（官方文档） |

---

## 十二、需要"官方支持"的完整厂商清单与协议分解（守溥 2026-09-10 令）

### 12-1 要求的清单（全部要一等公民）

Claude · GPT · Qwen（通义）· 智谱 GLM · DeepSeek · **MiMo（小米）** · **OpenCode** · **CommandCode**
（另含前面已列的 xAI Grok 与 StepFun 阶跃）。

### 12-2 关键架构结论：**协议少，服务商多**

这些"厂商"里**大多数不是新协议**，而是 **(协议 kind) × (base_url) × (密钥) × (模型清单) × (区域/渠道)** 的组合。
实测证据（取自一个装了全套预设的开源 coding agent，`internal/config/provider_presets.go`）：

| 预设 | **协议 kind** | base_url | 密钥环境 | 模型清单 |
|---|---|---|---|---|
| `mimo-api` | **openai** | `https://api.xiaomimimo.com/v1` | `MIMO_API_KEY` | `mimo-v2.5-pro`, `mimo-v2.5` |
| `mimo-anthropic` | **anthropic** | `https://api.xiaomimimo.com/anthropic` | 同上 | 同上 |
| `glm-cn`（智谱国内） | openai | `https://open.bigmodel.cn/api/paas/v4` | `GLM_API_KEY` | glm-5.2/5.1/5/5-turbo/5v-turbo/4.7… |
| `zai-global`（智谱海外） | openai | `https://api.z.ai/api/paas/v4` | `ZAI_API_KEY` | 同上 |
| `qwen-cn`（通义国内） | openai | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `QWEN_API_KEY` | qwen3.7-plus/max… **并托管他家模型**（MiniMax-M2.5、glm-5、kimi-k2.5） |
| `qwen-global` | openai | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | 同上 | 同上 |
| `deepseek-anthropic` | **anthropic** | （默认） | `DEEPSEEK_API_KEY` | deepseek-v4-pro/flash… |
| `opencode-zen-anthropic` | **zen-anthropic（自有变体）** | `https://opencode.ai/zen` | `OPENCODE_API_KEY` | **claude-sonnet-4-6 / opus-4-8 / haiku-4-5 / qwen3.x** |
| `stepfun-api` | openai | `https://api.stepfun.com/v1` | `STEPFUN_API_KEY` | step-3.7-flash / 3.5-flash… |

**读法（三条要紧的）**：

1. **MiMo 同时提供 OpenAI 与 Anthropic 两套兼容端点** → 一个厂商两条路。
2. **OpenCode Zen 提供的是 Claude 与 Qwen** → 它是**网关/聚合商**，不是模型厂商（所以"支持 OpenCode"= 支持一个**网关协议变体**）。
3. **CommandCode** 是**订阅制编程服务**：普通订阅走**它自家的 `/alpha/generate`**，只有付费 "Provider" 档才暴露 OpenAI 兼容面（`/provider/v1/chat/completions`）；密钥 `CC_API_KEY`，基址可覆盖 `CC_API_BASE`；社区靠代理把它转成 OpenAI/Anthropic 两端。
   → **两条路二选一**：① 只支持 Provider 档（OpenAI 兼容，工作量小，但用户得买那一档）；② 自研 `/alpha/generate` 适配（体验全，但要维护一个非标协议）。**须守溥定。**

⇒ **因此 llmrust 的正确形态是**：
**少数几个协议适配器**（`openai` / `anthropic` / `responses` / `google` / `ollama` / `zen`）
**＋ 一张服务商目录（数据）**（protocol + base_url + key env + 模型清单 + 区域/渠道变体）。
**不是**给每个厂商手写一个 provider。

### 12-3 目录要建的条目（按现状盘点）

| 厂商 | 上游 llmrust 现状 | 需要做的 |
|---|---|---|
| Claude(Anthropic) | ✅ 已有 `anthropic.rs` | 补缓存断点发送（卡 1） |
| GPT(OpenAI) | ✅ 已有 `openai.rs`/`compat.rs` | 无需（自动前缀缓存） |
| DeepSeek | ✅ 已有 `deepseek.rs` | 补 anthropic 端点预设 + 目录行 |
| Qwen 通义 | ❌ 无 | 预设行 ×2 区域（openai 兼容） |
| 智谱 GLM | ❌ 无 | 预设行 ×2 区域（openai 兼容） |
| **MiMo** | ❌ 无 | 预设行 ×2 协议（openai + anthropic） |
| **OpenCode** | ❌ 无 | **新协议变体 `zen`** + 预设行 |
| **CommandCode** | ❌ 无 | **待定**：Provider 档（openai 兼容）或 `/alpha/generate` 适配 |
| xAI Grok | ❌ 无（表里有 `xai/` 数据可抄） | 预设行（openai 兼容） |
| StepFun 阶跃 | ❌ 无 | 预设行 ×2（openai + anthropic） |

### 12-4 因此 §二 的表要加三列

| 新列 | 类型 | 说明 |
|---|---|---|
| `protocol` | `Protocol`（enum：OpenAi / Anthropic / Responses / Google / Ollama / Zen） | 走哪个适配器 |
| `base_url` | `&'static str` | 端点 |
| `key_env` | `&'static str` | 密钥环境变量名 |

> 这三列加上原有的模型/价格/能力列，**才构成"服务商目录"**；没有它们，目录只是"模型清单"，落不了地。

---

## 十三、守溥 2026-09-10 两项决定（已定，不再讨论）

### 13-1 CommandCode：走**官方付费档**，不碰社区代理

**决定**：CommandCode 只支持其**官方 "Provider" 档**（OpenAI 兼容面 `/provider/v1/chat/completions`），密钥 `CC_API_KEY`。
**不做**：自研 `/alpha/generate` 适配；**也不用**社区代理（`commandcode-proxy` 之类）。

### 13-2 总原则：**只支持官方**

即：**只接服务提供方自己的官方端点**，**不接第三方代理 / 转售 / 聚合**。

| 类别 | 处置 | 例子 |
|---|---|---|
| **各厂商自有官方端点** | ✅ 接 | `api.stepfun.com`、`open.bigmodel.cn` / `api.z.ai`、`dashscope.aliyuncs.com`、`api.deepseek.com`、`api.xiaomimimo.com`、`api.x.ai`、`opencode.ai/zen`、`commandcode.ai`（Provider 档） |
| **订阅制官方网关**（服务方自己的官方服务） | ✅ 接（官方端点） | OpenCode Zen、CommandCode |
| **第三方转售 / 托管行** | ❌ 不接 | `novita/*`、`deepinfra/*`、`azure_ai/*`、`bedrock/*`、`huggingface`、`nvidia` |
| **第三方聚合网关** | ❌ 不接 | OpenRouter、Vercel AI Gateway、kilocode、gmi |
| **社区代理 / 反代** | ❌ 不接 | `commandcode-proxy`、各类 `*-proxy` |

**连带影响（三条，必须记账）**：

1. **LiteLLM 表的角色降级**：它很多行是**转售行**（我们抽到的 StepFun 两行就是 `novita/` 与 `deepinfra/`）。
   → 按本原则，**它只能当"交叉校对尺"，不能当数据来源**——这与我们已定的"自建为主"一致，现在成为**硬要求**。
2. **官方文档访问成为硬前置**：三档价格、最小可缓存长度、TTL 档、自动/需标记，
   **必须查各家官方文档**才能落值；**该障碍已解除**——破法是用 **Node 自带 TLS** 抓取（见 §19-1 取数说明），
   已抓全十家（见 §十四）。
3. **我方代码现状要清账**：实测本项目代码里已引用 **`openrouter` 26 处**
   （`coding-agent/src/model/tests_thinking_formats.rs` 6、`coding-agent/src/model/mod.rs` 3、`yesagent-ai/src/model_registry/compat.rs` 3 …）。
   按本原则属"第三方聚合" → **需决定：移除，还是登记为在册债**。

### 13-3 与主项目的分工（本原则推导）

- **端点/模型/价格/能力** → 收进 llmrust 的**官方目录**（数据）；
- **我方代码只引用目录条目**，不再自己写端点与厂商分支（现行 `model/mod.rs` 的 10 处 provider 分支、`model_registry/compat.rs` 的域名表就是待清偿的债）。

---

## 十四、各家缓存能力的**官方口径**汇总（2026-09-10 逐家抓官方文档实测）

> 抓取方式：Node `fetch`（走 OpenSSL，绕开沙箱 schannel 限制）；落地文本存 `本地临时目录/*.txt` 可复核。
> **本节是表的取值依据**；凡与本节冲突的第三方数据（LiteLLM 等），**以本节为准**。

| 厂商 | 缓存**模式** | **读价倍率** | **写价倍率** | **最小可缓存** | **有效期/淘汰** | 匹配规则 | 数据来源 |
|---|---|---|---|---|---|---|---|
| **Anthropic** | 显式 **+ 自动**（自动断点占 4 槽之一） | **0.1×**（Fable 5.1 / Mythos 5.1 = **0.025×**） | **1.25×**(5m) / **2×**(1h) | 1024（Haiku 4096） | 5m/1h，**命中免费续期** | 逐字节前缀 | 官方 prompt-caching |
| **OpenAI** | **自动**（亦可显式断点） | **0.1×**（官方"最高省 90%"） | **GPT-5.6 起 1.25×**；老档不加价 | 按模型变（隐藏系统内容不计） | 默认 / **延长档最长 24h**；`prompt_cache_key` 分账 | 逐字节前缀 | 官方 prompt-caching |
| **DeepSeek** | **自动**，按**"缓存前缀单元"整块**匹配 | **≈0.02–0.033×**（$0.003/$0.15；$0.022/$0.66） | 无 | 按**固定 token 间隔落盘** | 磁盘缓存 | **单元整块匹配** | 官方 Models & Pricing |
| **阶跃 StepFun**（默认模型） | **自动**（> **256** token） | **0.2×**（1.35→**0.27 元**/M） | 无 | **256** | **LRU 动态**（无固定 TTL） | 逐字节前缀 | 官方定价页 + 《Prompt 缓存最佳实践》 |
| **通义（百炼）** | **隐式自动** ＋ **显式**（互斥） | **0.2×**（隐式）/ **0.1×**（显式） | 无 / **1.25×** | **1024** | 未定 / **5m（命中重置）** | 逐字节；**标记后 message >20 条则命不中** | 阿里云官方 Context Cache |
| **智谱 GLM** | **隐式自动**（无需配置） | **0.5×**（官方"标准价的 50%"） | 无 | 待查 | 待查 | 逐字节前缀 | 官方《上下文缓存》 |
| **xAI Grok** | **自动**（"consecutive requests…same starting messages"） | **0.25×**（grok-4.6：$2.00→**$0.50**；长上下文 $4.00→$1.00） | 无 | 待查 | 待查 | **逐字匹配** | 官方 Pricing |
| **MiMo（小米）** | 支持（"命中缓存"独立计价） | **≈0.0083×**（国内 ¥3.00→**¥0.025**；海外 $0.435→**$0.0036**） | 无 | 待查 | 待查 | 待查 | 官方 API 定价 |
| **CommandCode** | **网关**（OpenAI+Anthropic 双面，官方 Provider 档） | 按底层模型（官方"at cost"） | 按底层模型 | — | — | — | 官方 Provider/定价页 |
| **OpenCode Zen** | **网关**（官方 Zen，端点 `opencode.ai/zen/v1`） | 按模型（官方"zero markups"） | 部分模型有 | — | — | — | 官方 Zen 文档 |

### 由官方口径得出的**六条设计硬约束**

1. **`CacheMode` 必须是枚举**：`None | Auto | Explicit | Both`
   Anthropic=**Both**（显式+自动）；StepFun/DeepSeek/智谱/xAI/OpenAI/MiMo=Auto；通义=**Both**。
2. **读价必须逐模型存**（审核官 F-3 打回后已拆两列，**不许混口径**）：

   | 厂商 | **官方路径价**（A 级，定值用） | 他路参考价（C 级，**只作校对**） |
   |---|---|---|
   | **智谱 GLM** | **0.5×**（官方："通常为标准价格的 50%"） | 0.186×（LiteLLM `zai/glm-5.1` $0.26/$1.40） |
   | 通义（隐式） | **0.2×**（官方） | 同 |
   | 阶跃 StepFun | **0.2×**（官方 1.35→0.27 元） | 0.2×（LiteLLM 转售行） |
   | Anthropic / OpenAI | **0.1×**（官方；Fable/Mythos 0.025×） | 同 |
   | DeepSeek | **0.02–0.033×**（官方 $0.003/$0.15） | 同 |
   | xAI Grok | **0.25×**（官方 $2.00→$0.50） | 0.16×（LiteLLM `xai/grok-3`，**旧模型价，勿用**） |
   | MiMo | **0.0083×**（官方 ¥3.00→¥0.025） | — |

   实测跨度 **0.0083× ~ 0.5×（差约 60 倍）**。**任何全局默认倍率都是错的**；**定值只取 A 级列**。
3. **写价必须 `Option`**：只有 Anthropic 系（Claude 家族）与**部分 Qwen 模型**有写价；其余家为"写不加价"。
   （§二 表里 `cache_write_price_5m` 的"必做"**改为 `Option`**，与审核官第三条意见一致。）
4. **最小可缓存长度是数据**：**256**（阶跃）/ **1024**（通义、Anthropic Sonnet）/ **4096**（Anthropic Haiku）
   / DeepSeek 是**按固定间隔落盘的前缀单元**。→ 它**挂在模型上、不挂厂商上**（审核官 F-2 打回，见 §二十）。
5. **TTL 与淘汰是两件事**（审核官 F-4 打回后已拆，原稿把两者塞进一个"有效期"枚举）：

   ```rust
   /// 时长（能不能设、设多久）——与"怎么淘汰"正交
   pub enum Ttl { Fixed(Duration), Extended(Duration), None /*不支持指定*/ }
   /// 淘汰机制（谁来决定何时失效）
   pub enum Eviction { Expiry, Lru, DiskUnits, Unknown }
   ```
   | 厂商 | `ttl` | `eviction` |
   |---|---|---|
   | Anthropic | **Fixed(5m / 1h)**（命中免费续期） | Expiry |
   | 通义（显式） | **Fixed(5m)**（命中重置） | Expiry |
   | 通义（隐式）/ 智谱 | 无固定档（`None`） | Unknown（官方称"定期清理"） |
   | 阶跃 StepFun | **无固定档**（官方只说 LRU） | **Lru** |
   | DeepSeek | 无固定档 | **DiskUnits**（磁盘缓存单元） |
   | OpenAI | **Fixed(默认) / Extended(≤24h)** | Expiry |
6. **超长上下文要分档计价**：Grok（≤200k / >200k）、Gemini（≤200k / >200k）、GPT（≤272k / >272k）**价格不同** →
   表中 `context_window` 之外还需要**分档价格**（`pricing_tiers`），否则长上下文算错账。

### 附：CommandCode / OpenCode Zen 官方端点与形态

```
CommandCode（Provider 档，付费）：
  https://api.commandcode.ai/provider/v1/chat/completions   ← OpenAI 兼容
  https://api.commandcode.ai/provider/v1/messages           ← Anthropic 兼容
  https://api.commandcode.ai/provider/v1/models
  官方："bill at the underlying API rates… no hidden fees"；覆盖 Claude / GPT / Gemini / 开源全家
  套餐：Go $1 · GOAT $10 · Pro $20 · Provider $15（pay-as-you-go）· Max 10× $100 · Max 20× $200

OpenCode Zen（官方网关）：
  https://opencode.ai/zen/v1/models
  官方："an AI gateway"、"zero markups"；模型表含 Cache Read / Cache Write 两列（部分模型有写价）
  含免费模型（Big Pickle、MiMo-V2.5 Free 等）与**弃用模型表**（带日期）
```

---

## 十五、三家参考系的同构证据（pi / maka / LiteLLM 实读，2026-09-10）

> 结论先说：**三家在设计上高度一致**，我们的表列与它们对得上——这份设计不是我们自创。

### 15-1 计费字段：三家一致（这是最强的证据）

| 参考系 | 字段 | 位置 |
|---|---|---|
| **pi**（TS，`本地 pi 参考克隆`） | `cost { input, output, cacheRead, cacheWrite, cacheWrite1h? }`；注释明写 "**Only Anthropic reports this split**" | `packages/ai/src/types.ts:370-387` |
| **maka**（TS，`本地 maka 参考克隆`） | `PricingConfig { modelKey, inputUsdPer1M, outputUsdPer1M, **cacheReadUsdPer1M**, **cacheWriteUsdPer1M** }` | `packages/core/src/usage-stats/types.ts`（表在 `runtime/src/telemetry/builtin-pricing.ts`） |
| **LiteLLM**（Python） | `input_cost_per_token / output_cost_per_token / **cache_read_input_token_cost** / **cache_creation_input_token_cost**` | `model_prices_and_context_window.json` |

⇒ 我们表里的 `prompt_price / completion_price / cache_read_price / cache_write_price_5m / cache_write_price_1h(Option)` **与三家同构**；
其中 **1 小时档单独一列**正是 pi 的做法（"只有 Anthropic 会分开报"）。

### 15-2 数据来源：**生成 + 人工补充 + 用户覆盖**（maka 的注释给了标准答案）

maka `builtin-pricing.ts` 原话：
> "Access-path-specific pricing that **models.dev cannot represent**. The generated table is authoritative for ordinary overlapping keys;
> these rows only **fill gaps** or **preserve special plans whose provider price is not a public base rate**.
> **User pricing overrides still win**."

它的结构：
```
BUILTIN_PRICING = GENERATED_MODEL_PRICING（models.dev 生成）
                + LOCAL_PRICING_SUPPLEMENT（人工补充；只补缺口与"渠道价"）
                + 用户覆盖（优先级最高）
```
⇒ **这回答了 §九 第 1 条那个待定**：不是"自建 vs 对齐"，而是**两者结合**：
**models.dev 生成打底 → 人工补充"它表达不了的渠道/订阅价" → 用户可覆盖**。
（监控/复核层：主项目已有 `scripts/spcc/` 尺子，可对表做交叉核对。）

### 15-3 能力元数据字段（maka 的 `ModelMetadata`）

```
lifecycle: 'active'|'beta'|'alpha'|'deprecated'|'retired'   ← 含弃用（我们表里没有，该加）
contextWindow / inputLimit / maxOutputTokens
knowledgeCutoff / structuredOutput / lastUpdated
isFree                       ← "models.dev 计 0 成本"的免费档候选
capabilities / modalities
thinkingOptions              ← 每模型推理控制（镜像 models.dev reasoning_options）
```
⇒ 我们 §二 的表**要补 `lifecycle` 与 `isFree`**（前者管弃用、后者管免费档与"0 价"歧义）。

### 15-4 渠道/区域变体：三家的三种解法（我们选一种）

| 参考系 | 解法 | 例 |
|---|---|---|
| **maka** | **别名归一**到规范厂商 | `xai-oauth → xai`、`opencode-free → opencode`、`openai-codex → openai` |
| **pi** | **每家一个 provider 文件**（变体也各占一个） | `xiaomi-token-plan-cn`、`zai-coding-cn`、`qwen-token-plan-cn`、`opencode-go`、`minimax-cn` |
| **另一个开源 agent（Reasonix）** | **kind + base_url + key 的预设表** | 46 个预设，`mimo-api` / `mimo-anthropic` / `glm-cn` / `zai-global` … |

⇒ 建议 llmrust：**协议 kind 少而稳（适配器）＋ 目录行含 `protocol/base_url/key_env` ＋ 变体用"别名归一"**（maka 法，最省行数；pi 法最直白但要维护 40 个文件）。

### 15-5 缓存策略（pi 的完整形态，直接可抄）

```ts
type CacheRetention = "none" | "short" | "long";        // 三档（types.ts:102）
cacheControlFormat?: "anthropic";                       // 约定：打在 system / 最后一个工具 / 最后一条消息
const ttl = retention === "long" && compat.supportsLongCacheRetention ? "1h" : undefined;
compat.supportsCacheControlOnTools ? cacheControl : undefined
```
⇒ 我们 §三 的 `CacheRetention` 枚举 + `Caching` trait 关联常量与之同构；**再加官方的"模式枚举"**（§十四：Auto/Explicit/Both）即可覆盖十家差异。

---

## 十六、LiteLLM 实读（已克隆至 `本地 LiteLLM 参考克隆`，2026-09-10）

### 16-1 缓存注入的**生产级硬约束**（比厂商文档更硬，直接可抄）

来自 `litellm/integrations/anthropic_cache_control_hook.py`：

| 事实 | 值/原文 | 对 llmrust 的含义 |
|---|---|---|
| **断点上限** | `MAX_CACHE_CONTROL_BLOCKS = 4`；原话注释："Anthropic (and Bedrock Claude) reject requests with more than 4 cache_control breakpoints: **'A maximum of 4 blocks with cache_control may be provided.'**" | **必须在本地就挡住第 5 个**——正对应我们 §3-4 的 `Breakpoints<const N>`（越界编译期/构造期即失败） |
| **OpenAI 显式断点的起始版本** | `OPENAI_PROMPT_CACHE_BREAKPOINT_MIN_GPT_VERSION = (5, 6)` | OpenAI 只有 **GPT-5.6 起**才支持显式断点；之前的模型**只吃自动缓存** |
| **两套字段名并存** | `CACHE_BREAKPOINT_KEYS = ("cache_control", "prompt_cache_breakpoint")` | 目录里要按**协议**决定字段名，不能只写死一个 |
| **OpenAI 允许打断点的块类型** | `{"text","image","image_url","file","input_audio","input_text","input_image","input_file"}` | 白名单要进类型（枚举），不是任意块都能打 |
| **注入点 API** | 用户在 `completion params` 里给 `cache_control_injection_points`（含 `CacheControlInjectionPoint` / `CacheControlMessageInjectionPoint`） | 与 pi 的"三档 + 库内约定"相比，LiteLLM 给**显式注入点**；两者可并存：默认约定 + 可覆盖 |
| 共享注入助手 | `prompt_templates/common_utils.py::with_prompt_cache_breakpoint` | 注入逻辑**抽成一个函数**，不是各处手写 |
| 特殊路径识别 | 会识别 `api.openai.com` / `OPENAI_BASE_URL` / `OPENAI_API_BASE`，并对 Claude Code 的一次性子代理请求特殊处理 | 说明"按 host/渠道分流"是真实需求（对应我们 §12-2 的协议×渠道分解） |

### 16-2 它的 **Rust 版**（`litellm-rust/`）——加厂商的流程与编码标准

结构（`litellm-rust/crates/`）：`core`（路由+厂商）· `ai-gateway`（axum 宿主）· `config` · `python-bridge` · `python-interop`。

`ADDING_A_PROVIDER.md` 原文要点：
1. **路由**住在 `crates/core/src/<route>/`（`messages` 是参考实现）；宿主只调路由入口；
2. **入口** `mod.rs`：`pub async fn <route>(request) -> CoreResult<Response>` + `<route>_stream`；
3. **变换契约** `transformation.rs`：一个 `…ProviderConfig` trait（URL 构造 + 请求/响应变换），类型放 `types.rs`；
4. **厂商配置** `crates/core/src/providers/<provider>/<route>/transformation.rs`：**实现成 `const <PROVIDER>_<ROUTE>_CONFIG`**，并补 parity 单测；
5. **prepare + handler**：`prepare.rs` 解析 provider/model/凭证/鉴权头/URL 并变换请求；`handler.rs` 走共享 `client.rs` 发请求并变换响应。

**编码标准（原话，建议直接采纳为我们 §六 的正面表述）**：

> "Before writing new logic, look for an existing base to extend. …
> **Never copy an existing implementation and edit it in place, and never hand-roll a parallel version of logic a base already provides.**
> If you catch yourself writing a second copy of a pattern that exists twice already, **stop and extract a base instead** …
> **The test for a good abstraction is that adding the next provider is a few declarative lines, not a new file of duplicated flow.**
> Only diverge from the base when behavior is genuinely different, and say so explicitly in the PR."

> 另有一条分层纪律可照抄："hosts invoke the core entrypoint — … **Never add a provider handler to ai-gateway**"
> （对应我们"厂商适配只在 ai 层，不许散到别处"）。

### 16-3 厂商覆盖对照（LiteLLM ~120 家）

| 我们清单里的 | LiteLLM 有没有 |
|---|---|
| Anthropic / OpenAI / DeepSeek / Google / xAI / Moonshot / MiniMax | ✅ 都有独立目录 |
| **通义 Qwen** | ✅ `dashscope` |
| **智谱 GLM** | ✅ `zai`（海外）；国内走 `zai`/`openai_like` |
| 字节 / 腾讯 | ✅ `volcengine` / `tencent` |
| **StepFun 阶跃 / MiMo 小米 / OpenCode / CommandCode** | ❌ **都没有**（这四家只能我们自建，印证 §十一 的"覆盖缺口"） |
| 第三方（OpenRouter / Novita / DeepInfra / Vercel） | ✅ 有，但按守溥"只支持官方"令**不接** |

---

## 十七、Rust 红利怎么落地：这张表为什么在 Rust 里**更强**（不是"也能做"）

> 三家参考系里：**LiteLLM 是 Python、pi 与 maka 是 TypeScript**。
> 它们在"能力/价格"这件事上**只能**用「运行时字典 + 运行时 if + 布尔旗标」表达——
> 那不是设计选择，是语言限制。**llmrust 是 Rust，可以把同一份事实表达成"编译期可证"的形状。**
> 下面每一条都给可落地的代码形状，并标出**它替我们挡掉了什么**。

### 17-1 能力 = trait + 关联常量（编译期能力表）——**已按审核官 F-2 收窄**

```rust
/// 只放"协议级"常量：同一协议下所有模型一致的那些。
/// **逐模型会变的数字（最小可缓存长度、TTL 档、是否计写价）一律进 CacheSpec 数据**，
/// 否则表达不了 Anthropic Haiku 4096 vs Sonnet 1024 这类例外（原稿之错）。
pub trait Caching {
    const MAX_BREAKPOINTS: usize;   // 协议级：Anthropic 与 Bedrock Claude = 4
}
pub trait Reasoning { type Effort; }
pub trait ToolCalling { const PARALLEL: bool; }
pub trait Vision {}

/// 逐模型事实（随 ModelSpec 走，见 §3-2 / §18-3）
pub struct CacheSpec {
    pub mode: CacheMode,             // Auto | Explicit | Both | None
    pub min_cacheable_tokens: u32,   // Haiku 4096 / Sonnet 1024 / 阶跃 256 / 通义 1024
    pub ttl: Ttl,
    pub eviction: Eviction,
    pub write_billed: bool,          // 写缓存是否计费（Anthropic 系 / 部分 Qwen = true）
}
```

**替我们挡掉**：TS/Python 里"这家其实不支持长 TTL / 这个模型的最小长度不一样"只能在**运行时**发现
（发出去、被上游拒、或静默降级）；Rust 里**不实现 `Caching` 的载体，在类型上就没有断点这个能力**，
而**逐模型差异走数据 + `deny_unknown_fields` + const 断言**，写错就编译/解析失败。

### 17-2 typestate / marker trait：**不支持的组合，编译不过**

```rust
pub struct Provider<P: Protocol, C: Capability> { /* ... */ }

impl<P: Protocol, C: Caching> Provider<P, C> {
    /// 只有"实现了 Caching"的载体才有这个方法
    pub fn with_cache_retention(self, r: CacheRetention) -> Self { /* ... */ }
}

// 用了不支持缓存的厂商 → 编译期报错，而不是运行时 Unsupported { feature: String }
//   let p = PROVIDER_STEPFUN.with_cache_retention(Long);  // 仅当 StepFun 实现 Caching 才可编译
```

**替我们挡掉**：llmrust 现在用 `LlmError::Unsupported { feature: String, message: String }`——
**字符串能力**会漂（写错一个字母就静默不生效）。Rust 让"缺能力"变成**类型上不可达**。

### 17-3 断点上限进类型（LiteLLM 在运行时查，我们在构造期挡）

```rust
/// 断点数量写进类型；`MAX` 来自该厂商的 `Caching::MAX_BREAKPOINTS`
pub struct Breakpoints<const N: usize, P: Caching>([BlockRef; N]);

impl<const N: usize, P: Caching> Breakpoints<N, P> {
    pub fn push(self, b: BlockRef) -> Breakpoints<{ N + 1 }, P> { /* ... */ }

    /// 只在 N <= P::MAX_BREAKPOINTS 时可用（越界 → 编译期失败）
    pub fn validate(self) -> Self where Assert<{ N <= P::MAX_BREAKPOINTS }>: True { self }
}
```

**替我们挡掉**：LiteLLM 是运行时 `MAX_CACHE_CONTROL_BLOCKS = 4` 检查，
一旦漏检就**把非法请求发给 Anthropic**，换来一句 `A maximum of 4 blocks with cache_control may be provided.`
→ 我们**在本地就把第 5 个断点变成编译错误**。

### 17-4 表在**构建期**落地，运行时零解析、零字典查找

```rust
// 源数据（可对齐 models.dev）→ xtask 生成器 → 强类型常量
pub const MODEL_CATALOG: &[ModelSpec] = include!("model_catalog.generated.rs");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]          // 表里写错字段 → 解析期炸，不静默失效
pub struct ModelSpec { /* ... */ }
```

- **查表**用 `phf`（完美哈希）或有序 `const` 切片 + 二分：**O(1)、零分配、无反射**（Rust 独有）；
- 对比 LiteLLM：**启动时加载 2.36 MB JSON 再建字典**。

### 17-5 穷尽 match：新增协议/能力，编译器逼你改完所有分支

```rust
pub enum Protocol { OpenAi, Anthropic, Responses, Google, Ollama, Zen }

fn build_body(p: Protocol, req: &ChatRequest) -> Body {
    match p {                                  // 新增一个协议 → 所有 match 点全部报错
        Protocol::Anthropic => build_anthropic(req),
        /* ... */
    }
}
```

**替我们挡掉**：LiteLLM/pi 加一家新协议时，**漏改某处分支不会有任何提示**（Python/TS 都只能在运行时暴露）。

### 17-6 表内不变量用 **const 断言**（编译期自证）

```rust
/// 声明了缓存读价 ⇒ 必须同时有 cache_spec；否则编译不过
const fn assert_cache_price_consistent(s: &ModelSpec) {
    if s.cache_read_price.is_some() { assert!(s.cache_spec.is_some()); }
}

/// 目录整体自检（生成器同样跑一遍，构成"双保险"）
const _: () = { let mut i = 0; while i < MODEL_CATALOG.len() { assert_cache_price_consistent(&MODEL_CATALOG[i]); i += 1; } };
```

**替我们挡掉**：三家参考系里，"价格表与能力表互相矛盾"**只能靠人眼或运行时校验**发现。

### 17-7 错误用 enum 穷尽（而非字符串 / 异常层级）

```rust
pub enum CapabilityError {
    NotSupported { capability: Capability, provider: ProviderSlug },
    BreakpointLimitExceeded { max: usize, got: usize },   // 本地就报，不发出去
    TooShortToCache { min: u32, got: u32 },               // 小于 MIN_CACHEABLE_TOKENS
}
```
- LiteLLM 是**异常层级 + 字符串**；pi 是 TS union；
- Rust 的 enum 让**下游（我们的 yesagent-ai）在编译期就被强制处理每一种失败**。

### 17-8 新类型（newtype）防串账 + 稳定的下游契约

```rust
pub struct ModelId(&'static str);
pub struct ProviderSlug(&'static str);
pub struct Money(f64);

#[non_exhaustive] pub enum CacheMode { None, Auto, Explicit, Both }
```
`#[non_exhaustive]` 让我们（下游）在 llmrust 加新变体时**被强制重新编译处理**，而不是静默 fallback。

### 17-9 流式与零拷贝：Rust 在"请求整形"上还有一手

```rust
// 断点标记是"叠在已有块上"的，不需要把整棵消息树深拷贝一份再改
pub fn with_cache_control<'a>(block: &'a mut Block, cc: CacheControl) -> &'a Block
```
对比 LiteLLM 的 `anthropic_cache_control_hook.py`：它用 `copy.deepcopy(...)` 再改；
pi/maka 走 JS 对象展开。**Rust 可以就地把标记叠上去（`&mut`），或零拷贝地借用构造**——
请求整形路径上省掉一次整树深拷贝（我们主项目的请求就带整段历史）。

### 17-10 诚实的代价（不是免费）

1. **构建期生成**多一步（`build.rs`/xtask）——换来运行时零解析；
2. **trait-per-capability 会影响 `dyn` 兼容**——需要时改用"能力枚举 + 关联常量"（§17-1 已可覆盖），或给需要 `dyn` 的路径留一个 `CapabilitySet` 位集；
3. **编译期断言的报错信息**对使用者不友好——须在类型上写清 doc 注释（"为什么编译不过"）；
4. **生成器与 CI 门**（数据 lint）本身要维护——但这是"一次投入、长期免漂"。

---

### 一句话对照（给上游的一句话）

> LiteLLM / pi / maka 的能力与价格是**运行时数据**（字典/对象字段 + 运行时 if）；
> llmrust 可以把它做成**编译期事实**：能力是 trait、策略是 enum、断点是类型、表是构建期常量、
> 失败是穷尽 enum——**同一条错误在它们那里要等运行时，在我们这里过不了编译**。

---

## 十八、**「模型」与「接入路径」是两个正交维度**（守溥 2026-09-10 指正的落地）

### 18-1 事实（实测，纠正我原稿的一处概念错误）

我原稿把 `dashscope/glm-5.1` 标成"通义 GLM"——**错了**。核实后：

- `dashscope/` 前缀下 **45 条**，其中**不属于阿里的**有：`deepseek` **3** 条、`glm` **2** 条、`kimi` **1** 条；
- **同一个 `glm-5.1` 存在于两条路径**：
  | 键 | `litellm_provider` | 输入/输出/缓存读 |
  |---|---|---|
  | `dashscope/glm-5.1`（经**阿里百炼**） | `dashscope` | $1.40 / $4.40 / $0.26 |
  | `zai/glm-5.1`（**智谱官方**） | `zai` | $1.40 / $4.40 / $0.26 |
- ⇒ **表键前缀 = 「接入路径」（谁提供接入），不是「模型厂商」（谁造的模型）**。
  GLM 是**智谱造的**；阿里百炼只是**能接入它的平台之一**。

### 18-2 为什么这条必须进设计（不是抠字眼）

1. **同一个模型，经不同路径接入，价格与能力可能不同**：
   - maka 的定价表注释原话：*"**Access-path-specific pricing that models.dev cannot represent**"* ——它专门用 `LOCAL_PRICING_SUPPLEMENT` 补这类"渠道价"；
   - 例：maka 里有 `zai-coding-plan:glm-4.7` 这种**订阅渠道键**，与官方 API 价不同。
2. **同一模型不同路径，缓存行为可能不同**（自动/显式、最小长度、TTL 都可能各说各话）。
3. **我们的"只支持官方"令天然简化了这件事**：只收**厂商官方路径**与**官方网关**，
   第三方转售（`novita/`、`deepinfra/`、`azure_ai/`、`bedrock/`）与聚合（`openrouter/`、`vercel_ai_gateway/`）**一律不接**。

### 18-3 目录结构（两维分开建模，不要混成一张表）

```rust
/// 维度一：模型本体（谁造的、多大、有什么能力）
pub struct ModelSpec {
    pub id: ModelId,                 // 规范模型名（如 glm-5.1）
    pub vendor: VendorSlug,          // 造它的厂商（如 zhipu）
    pub context_window: u32,
    pub max_output: u32,
    pub caching: Option<CacheSpec>,  // 模型自身能力
    pub reasoning: Option<ReasoningSpec>,
    pub tool_calling: ToolCalling,
    pub lifecycle: Lifecycle,        // active/beta/deprecated/…（照 maka 的 ModelMetadata）
}

/// 维度二：接入路径（从哪接、怎么认证、能接哪些模型）
pub struct AccessPath {
    pub id: PathId,                  // 如 zai-global / glm-cn / dashscope-cn / mimo-anthropic
    pub protocol: Protocol,          // OpenAi | Anthropic | Responses | Google | Ollama | Zen
    pub base_url: &'static str,
    pub key_env: &'static str,
    pub region: Region,              // cn / global / …
    pub channel: Channel,            // api / coding-plan / token-plan / provider-tier
    pub models: &'static [ModelId],  // 该路径能接哪些模型
    /// 该路径的**价格覆盖**（缺省 = 用模型本体价；有则覆盖，含缓存三档）——照 maka 的"渠道价"
    pub price_override: Option<&'static PriceTiers>,
}

/// 关系：路径 ⇄ 模型 是多对多
pub const ACCESS_PATHS: &[AccessPath] = &[ /* zai-global, glm-cn, dashscope-cn, … */ ];
```

**为什么必须分开**（三条硬理由）：
1. **同一个模型可以有多条官方路径**（例：GLM-5.1 既有智谱官方 `z.ai`/`bigmodel.cn`，也在百炼上被托管——但我们**只收官方那条**）；
2. **同一模型在不同路径下的价格/能力可能不同**（订阅渠道 vs 按量、区域差异、缓存行为差异）；
3. **厂商 ≠ 路径**：`dashscope` 是平台、`zhipu` 是厂商；把两者混在一列，**"通义有 GLM"这种错就会混进表**。

### 18-4 三家参考系其实都这么分（只是形式不同）

| 参考系 | 模型维度 | 路径维度 | 归一手段 |
|---|---|---|---|
| **maka** | `ModelMetadata`（含 `capabilities`/`thinkingOptions`/`lifecycle`） | `llm-connections`（连接=路径） | `GENERATED_METADATA_PROVIDER_ALIASES`：`xai-oauth→xai`、`opencode-free→opencode`、`openai-codex→openai` |
| **pi** | `<vendor>.models.ts`（数据） | `<vendor>.ts` 适配器（`createProvider({id,name,baseUrl,auth})`） | 变体各占一个 provider 文件（`xiaomi-token-plan-cn` 等） |
| **LiteLLM** | 表里的模型键 | `litellm_provider` + `custom_llm_provider` | 前缀即路径（所以出现 `dashscope/glm-5.1` 这种"路径/模型"写法） |

⇒ **llmrust 采用「两维分开 + 别名归一」**：模型表只写"谁造的、多大、有什么能力"；
路径表只写"从哪接、怎么认证、能接哪些、价格是否覆盖"。

---

## 十九、**数据来源与取数方法（事无巨细，可逐条复核）**

> 本稿所有数字与结论都可追溯到下面某一条；**凡未列出的，就是"待查"，不许当已证**。
> 取数日期：**2026-09-10**（守溥令当日）。

### 19-0 取数纪律（先说规矩）

| 级别 | 含义 | 用法 |
|---|---|---|
| **A 官方** | 厂商自己站点/官方文档 | **可进判据、可进目录定值** |
| **B 我方实测** | 我自己在树上跑出来的（命令+输出） | 可进判据 |
| **C 第三方成品** | LiteLLM 表 / pi / maka / 别的开源项目 | **只作交叉校对与结构参考，不作定值来源** |
| **D 二手转述** | 博客/搜索摘要/社区 | **只作线索，不进判据** |

### 19-1 官方文档（A 级）——逐条来源与取数方式

> **取数方式说明**：本执行环境里 `curl` / PowerShell 抓 HTTPS 会因 **Windows 证书库不可达**失败
> （实测 `curl: (35) schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS`）；
> 且 DNS 常被本机代理接管（`platform.stepfun.com → 198.18.0.71`，Clash 类 fake-IP 段）。
> **破法**：改用 **Node 自带 TLS**（`node fetch`）抓取 → 落地为文本文件（存 `本地临时目录\` 与 `本地抓取产物目录（不入库）\`）。
> 抓取脚本：`（本地单页抓取脚本）`（单页 + 关键词抽取）与 `（本地批量抓取脚本）`（批量）。

| # | 数据点 | 值 | 来源（URL） | 落地文件 | 取数方式 |
|---|---|---|---|---|---|
| 1 | 阶跃**缓存是自动的**、最小 256、读价 **20%**、LRU、官方建议"保持前缀稳定" | 见 §十四 | `https://platform.stepfun.com/docs/zh/guides/developer/prompt-cache` | `本地临时目录\stepfun-cache.txt` | `node grab.js <url> stepfun-cache 缓存,命中,…` |
| 2 | 阶跃 **step-3.7-flash 定价**：输入 1.35 元 / **命中 0.27 元** / 输出 8.1 元（每 M） | 0.2× | `https://platform.stepfun.com/docs/zh/guides/pricing/details` | `本地抓取产物目录（不入库）\stepfun-price.txt` | `node fetchmany.js` |
| 3 | Anthropic：显式**+自动**、读 0.1×（Fable/Mythos 0.025×）、写 1.25×(5m)/2×(1h)、命中免费续期、4 断点 | 见 §十四 | `https://platform.claude.com/docs/en/build-with-claude/prompt-caching` | `本地抓取产物目录（不入库）\anthropic-cache.txt`（62k 字符） | Node fetch |
| 4 | OpenAI：自动、**最高省 90%**、**GPT-5.6 起写 1.25×**、`prompt_cache_key`、延长保留 **≤24h** | 见 §十四 | `https://developers.openai.com/api/docs/guides/prompt-caching` | `本地抓取产物目录（不入库）\openai-cache.txt`（53k） | Node fetch |
| 5 | DeepSeek：**缓存前缀单元**整块匹配、按固定间隔落盘、双 base URL（OpenAI/Anthropic） | 见 §十四 | `https://api-docs.deepseek.com/guides/kv_cache`、`/quick_start/pricing` | `本地临时目录\deepseek-cache.txt`、`off\deepseek-price.txt` | Node fetch |
| 6 | DeepSeek 价格：flash 命中 **$0.003** / 未命中 $0.15 / 输出 $0.6（per 1M）；context **1M**、max output **384K** | ≈0.02× | 同上 `quick_start/pricing` | 同上 | Node fetch |
| 7 | 通义：**隐式自动（20%）+ 显式（125% 写 / 10% 读 / 5 分钟）**、最小 **1024**、三种协议都支持、`prompt_tokens_details.cached_tokens` | 见 §十四 | `https://help.aliyun.com/zh/model-studio/context-cache` | `本地临时目录\qwen-cache.txt`（36k） | Node fetch |
| 8 | 智谱：**隐式自动**、命中 **"通常为标准价格的 50%"**、差异化计费（新内容标准价） | 0.5× | `https://docs.bigmodel.cn/cn/guide/capabilities/cache` | `本地临时目录\zhipu-cache.txt` | Node fetch |
| 9 | xAI：**自动**、要求"**match exactly**"、缓存按折扣计费；grok-4.6 定价 输入 $2.00 / **缓存 $0.50** / 输出 $6.00（≥200k 翻倍） | 0.25× | `https://docs.x.ai/developers/advanced-api-usage/prompt-caching`、`https://docs.x.ai/developers/pricing` | `本地抓取产物目录（不入库）\xai-cache2.txt`、`off\xai-price.txt` | Node fetch |
| 10 | MiMo：`mimo-v2.5-pro` 国内 ¥3.00 → **命中 ¥0.025** / 输出 ¥6.00；海外 $0.435 → **$0.0036** / $0.87 | ≈0.0083× | `https://mimo.mi.com/docs/price/pay-as-you-go` | 搜索返回正文（该页为 SPA，Node 直抓只得 16 字符） | `web_search` 返回的正文 |
| 11 | CommandCode：Provider 档端点三枚、"bill at the underlying API rates"、覆盖 Claude/GPT/Gemini/开源、套餐 Go/GOAT/Pro/**Provider $15**/Max | 见 §十四附 | `https://commandcode.ai/docs/provider`、`/docs/resources/pricing-limits` | `本地抓取产物目录（不入库）\cmdcode-provider.txt`、`off\cmdcode-price.txt` | Node fetch |
| 12 | OpenCode Zen：官方自述 "**an AI gateway**"、"zero markups"、端点 `opencode.ai/zen/v1` | 见 §十四附 | `https://opencode.ai/docs/zen/` | `本地抓取产物目录（不入库）\opencode-zen.txt` | Node fetch |

### 19-2 第三方成品（C 级）——来源、以及**用它做了什么/没做什么**

| 来源 | 位置 | 我用它得到了什么 | **明确不做**的 |
|---|---|---|---|
| **LiteLLM 价格/能力总表** | `gh api repos/BerriAI/litellm/contents/model_prices_and_context_window.json -H 'Accept: application/vnd.github.raw'` → `本地临时目录\litellm-prices.json`（**2.25 MB / 3886 条**，PS 需 `ConvertFrom-Json -AsHashtable`，否则"大小写重名键"报错） | 交叉校对：xAI 0.16×(grok-3)、StepFun 0.2×(转售行)、GLM/通义的价格量级；**证明"读价逐家不同"** | **不用它做定值**（其行多为第三方转售/托管，见 §18-1） |
| **LiteLLM 源码（已克隆）** | `本地 LiteLLM 参考克隆`（`git clone --depth 1`，**10,697 文件**，约 120 家 provider 目录） | 缓存注入**硬约束**：`litellm/integrations/anthropic_cache_control_hook.py`（`MAX_CACHE_CONTROL_BLOCKS = 4` 及上游报错原文、`OPENAI_PROMPT_CACHE_BREAKPOINT_MIN_GPT_VERSION=(5,6)`、`CACHE_BREAKPOINT_KEYS`、块类型白名单）；provider 目录覆盖清单（§16-3） | 不照抄它的 Python 形态（守溥令：要用 Rust 红利） |
| **LiteLLM 的 Rust 版** | `本地 LiteLLM 参考克隆\litellm-rust\`（`ADDING_A_PROVIDER.md`、`crates/{core,ai-gateway,config,python-bridge,python-interop}`） | 加厂商**流程**（路由/契约/厂商 `const` 配置/prepare+handler）与**编码标准原文**（§16-2） | 同左 |
| **pi** | `本地 pi 参考克隆`（真身；`本地 reference/pi 副本` 是**空壳**，只有 23 个文件，`packages/agent` 无 `src`） | 计费字段 `cost{input,output,cacheRead,cacheWrite,cacheWrite1h?}`（`packages/ai/src/types.ts:370-387`）；缓存三档 `CacheRetention`（`:102`）与打点约定（`packages/ai/src/api/anthropic-messages.ts` `getCacheControl`）；**~40 家 provider 各一对文件**（`providers/<v>.models.ts` + `<v>.ts`）；数据由 `scripts/generate-models.ts` 从 **models.dev** 生成 | 不照抄 TS 的运行时表达 |
| **maka** | `本地 maka 参考克隆`（真身） | 定价表**两级结构**（生成 + 人工补充 + 用户覆盖）：`packages/runtime/src/telemetry/builtin-pricing.ts`；能力元数据字段：`packages/core/src/model-metadata.ts`（`lifecycle/contextWindow/maxOutputTokens/knowledgeCutoff/isFree/capabilities/thinkingOptions`）；**路径别名归一** `GENERATED_METADATA_PROVIDER_ALIASES` | 同左 |
| **另一个开源 agent（Reasonix）的预设表** | `gh api repos/esengine/DeepSeek-Reasonix/contents/internal/config/provider_presets.go -H 'Accept: application/vnd.github.raw'` → `本地临时目录\reasonix-presets.go`（40.6 KB，**46 个预设**） | **协议 × 渠道 × 区域**的分解证据：`mimo-api`/`mimo-anthropic`、`glm-cn`/`zai-global`、`qwen-cn`/`qwen-global`、`opencode-zen-anthropic`；各家 base_url 与 key_env（§12-2） | 不作定值来源 |

### 19-3 我方实测（B 级）——本项目树上的证据

| 结论 | 命令（可复跑） | 位置 |
|---|---|---|
| llmrust **有电表、没开关** | `git grep -c cache_control/cache_read/cache_creation` 于 `本地 llmrust 检出（0.1.3）` | `src/providers/anthropic.rs:403-443`、`compat.rs:150-160,395-403` |
| Anthropic 请求体**无缓存字段**、`extra` 被忽略 | 读 `fn build_body` | `src/providers/anthropic.rs` `AnthropicRequest` 字段表（§二 表） |
| 内容块**只有 Text/ImageUrl** | 读 `pub enum ContentPart` | `src/types.rs:127` |
| `system` **只能是字符串** | 读结构体 | `src/providers/anthropic.rs:45` |
| 上游**主干也没有**发送侧 | `gh api repos/llmrust/llmrust/contents/src/providers/anthropic.rs`（base64 解码）→ `cache_control`=0、`ephemeral`=0 | 主干 `96bc1be` |
| 本地 llmrust 与远端**完全对齐** | `git fetch` 后 `git rev-list --left-right --count origin/main...main` = `0 0` | `本地 llmrust 检出（0.1.3）` |
| 主项目**默认模型** = `stepfun/step-3.7-flash` | 读 `config/settings.json` | `yesagent/config/settings.json` |
| 我方代码已引 `openrouter` **26 处**（按"只支持官方"属待清账） | `Select-String -Path crates,config -Pattern openrouter` | `coding-agent/src/model/{mod.rs,tests_thinking_formats.rs}`、`yesagent-ai/src/model_registry/compat.rs` |
| 我方前缀稳定性 5✅2⚠️ | 见前次审计（`active: Vec` 而非 HashMap 遍历等） | `crates/agent-core/src/tool/registry.rs:11-13,36-46,65-100` |

### 19-4 尚缺（**不许猜**，等来源）

| # | 缺什么 | 为什么缺 | 谁补 |
|---|---|---|---|
| A | 智谱/阶跃**自有 API** 行的上下文窗口与三档价格（LiteLLM 只有转售行，属 C 级） | 需查各自官方定价页 | 我方继续抓（智谱定价页为 SPA/404，需换址） |
| B | MiMo 的缓存**模式/最小长度/TTL** | 官方页为 SPA，Node 直抓只得 16 字符 | 需换址或换方式 |
| C | 各家**最小可缓存长度**（智谱、xAI、MiMo） | 同上 | 官方文档补齐 |
| D | CommandCode **逐模型费率表** | 其 `/docs/models` 404；费率在 `/docs/resources/pricing-limits` 的"Model pricing. At cost."段（搜索已见正文，待逐行抓全） | 我方继续抓 |
| E | OpenAI **各模型的最小可缓存长度具体值** | 官方只写"varies by model" | 官方 models 页 |

---

## 二十、默认模型 `stepfun/step-3.7-flash` 缓存能力**结论（已定案）**

> **审核官 F-1 打回后重写**：原 §十 写在官方文档抓到**之前**，因此把"官方站不可达、A–D 尚缺"当现状，
> 与 §十四 自相矛盾。**本节以已抓到的官方口径定案**；同时修正编号错位（原 §十 排在 §十九 之后）。

**为什么要单列**：它是主项目默认模型；若它不支持缓存，AC-16 的"打点止血"在默认配置下等于零。

### 20-1 定案（A 级官方来源）

| 项 | 值 | 来源 |
|---|---|---|
| 缓存**模式** | **Auto（自动，无需 `cache_control` 标记）**：请求 **> 256 Token** 时自动启用 | 官方《Prompt 缓存最佳实践》`platform.stepfun.com/docs/zh/guides/developer/prompt-cache` |
| **读价** | **0.2×** —— `step-3.7-flash` 输入 **1.35 元/M** → **命中 0.27 元/M** | 官方定价页 `.../docs/zh/guides/pricing/details` |
| **输出价** | **8.1 元/M** | 同上 |
| **写价** | **无**（未命中→推理完→前缀自动入缓存，不加价） | 官方《Prompt 缓存最佳实践》 |
| **最小可缓存长度** | **256 Token**（`min_cacheable_tokens = 256`） | 同上 |
| **`ttl`** | **无固定档**（官方未给 5m/1h） | 同上 |
| **`eviction`** | **LRU**（高峰期更易被逐出；官方建议"尽可能保持 Prompt 前缀稳定"） | 同上 |
| 命中判据 | `usage.cached_tokens` | 同上 |
| 官方实测效果 | 591 输入 → 命中 512 → 计费 79（**降 88%**） | 同上 |
| 抓取日 | **2026-09-10**（Node fetch；落地 `本地临时目录\stepfun-cache.txt`、`off\stepfun-price.txt`） | §十九 |

⇒ **结论：支持（Auto / 0.2× / 256 min / LRU）**。**AC-16 在默认模型上不会白干**；
且因为是**自动缓存**，主项目在默认配置下**只需保前缀稳定**，**连断点都不用打**。

### 20-2 残留待查（仅剩这些，且都不影响上面定案）

| # | 待查 | 为什么 |
|---|---|---|
| A | OpenAI 兼容端点 `/v1/chat/completions` 是否同样自动缓存 | 只有 Anthropic 端点的第三方实测（`cache_read_input_tokens=128`） |
| B | 缓存支持模型清单（官方页有此表，抓取时被截断） | 用于目录行覆盖范围 |

（原 §十 的"官方站不可达"已于本次抓取解决：破法是**改用 Node 自带 TLS**，见 §19-1 取数说明。）
