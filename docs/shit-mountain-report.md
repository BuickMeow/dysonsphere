# 🏔️ 屎山指数报告 (Shit Mountain Index Report)

**项目名称**: Dysonphere（戴森球）合成器引擎  
**分析日期**: 2026-05-02  
**代码规模**: 约 2000 行 Rust（不含示例）  

---

## 📊 Overall Assessment

- **屎山指数**: **68/100**（中高等级，结构性问题与业务逻辑 Bug 并存）
- **主要问题类别**:
  1. **增益链失控**（默认电平过高、多音符叠加无限制器）
  2. **包络解析默认值陷阱**（SF2 Release 时间接近零）
  3. **架构半成品状态**（Stereo TODO、Damper 未实现、RPN Bug）
- **风险等级**: 🔴 **High**（影响音频输出质量，AI 多次修改失败）

---

## 🔍 详细发现 (Detailed Findings)

### 问题 1：默认电平过高 —— 增益链缺少统一规划

- **问题描述**: 从采样数据到最终输出的整条增益链没有任何主控衰减或限幅机制。SF2/SFZ 默认解析出的 `volume` 均为 `1.0`（全音量），而 WAV/SF2 采样数据本身通常已经是归一化到 `-1.0~1.0` 的。叠加多个 Voice 时直接做加法混音，16 个音符同时响峰值可达 `16.0`，输出严重削波。所有示例程序在写文件前都**被迫手动做外部归一化**（`0.9 / max`），这已经说明内部电平完全不可控。
- **影响文件**:
  - `dysonphere-core/src/voice.rs:28,41,79`
  - `dysonphere-core/src/synth.rs:414,430,485-496`
  - `dysonphere-soundfont/src/sf2.rs:417`
  - `dysonphere-soundfont/src/sfz.rs:71`
- **严重程度**: 🔴 Critical（直接导致削波失真）
- **修复建议**:
  1. 在 `Synthesizer::read_samples` 中加入**软限制器**（soft clipper 或 tanh 限幅）或**动态归一化**（每帧统计峰值后做自适应衰减）。
  2. 为 `Voice` 增加一个 `master_volume` 衰减系数（如 `0.3`），在 `process()` 中作为最终乘法因子。
  3. 长期来看，建议将增益链拆分为独立的 `GainStage` 模块，便于不同音色库做 per-soundfont 响度校准。

```rust
// voice.rs:79 当前代码
pub fn process(&mut self) -> f32 {
    let sample = self.sampler.process();
    self.envelope.process();
    sample * self.envelope.value() * self.volume  // ← 此处 sample 和 volume 都接近 1.0
}
```

```rust
// synth.rs:485-496 Mono 混音，无限制器
for sample in buffer.iter_mut() {
    let mut mix: f32 = 0.0;
    while i < self.voices.len() {
        mix += self.voices[i].1.process();  // ← 直接累加，无上限
        // ...
    }
    *sample = mix;  // ← 可能远超 1.0
}
```

---

### 问题 2：默认 Release 音过低 —— SF2 解析默认值陷阱

- **问题描述**: 用户反馈 Release 音过低（释放太快）。表面看 `EnvelopeDescriptor::default()` 中 `release: 2.0` 似乎合理，但 **SF2 解析路径根本不使用这个默认值**。在 `sf2.rs` 中，`env_release` 的合并逻辑为 `timecents_merge(-12000, preset, instrument)`，当预设和乐器均未指定 release 时，结果为 `-12000` timecents，转换为秒数为 `2^(-10) ≈ 0.00098` 秒。尽管 `Envelope::new` 中有 `.max(0.001)` 和 `.max(0.15)` 的保护，但这只能把 release 时间从 0.001 秒拉到 0.15 秒，**对于钢琴、弦乐等需要自然尾音的音色来说仍然过短**。AI 多次修改失败，很可能是因为只改了 `EnvelopeDescriptor::default()` 的 release 值，**没有触及 SF2 解析路径中 `timecents_merge` 的默认参数**。
- **影响文件**:
  - `dysonphere-soundfont/src/sf2.rs:438-440,555-561`
  - `dysonphere-core/src/envelope.rs:59,88-91`
  - `dysonphere-soundfont/src/types.rs:31`
- **严重程度**: 🔴 Critical（影响音色自然度，AI 多次修复未果）
- **修复建议**:
  1. **根因修复**：在 `timecents_merge` 或 `build_presets` 中，为 `env_release` 设置一个合理的**绝对下限**（如 `0.5` 秒或 `1.0` 秒），覆盖 SF2 默认值 `-12000`。注意这只需要改 `env_release` 的处理逻辑，不能影响 attack/decay 等其他参数。
  2. 在 `Envelope::new` 中，将 `max(0.15)` 提升为可配置的全局常量（如 `DEFAULT_RELEASE_FLOOR`），避免硬编码。
  3. 增加一个 `EnvelopeDescriptor::default_for_synth()` 工厂函数，区分 "文件解析默认值" 和 "合成器业务默认值"。

```rust
// sf2.rs:555 —— 默认值 -12000 timecents 对应 0.001 秒
fn timecents_merge(_default: i16, preset: Option<i16>, instrument: Option<i16>) -> i32 {
    i32::from(instrument.unwrap_or(-12000)) + i32::from(preset.unwrap_or(0))
}

// sf2.rs:564 —— 转换结果
fn timecents_to_seconds(tc: f32) -> f32 {
    if tc <= -32768.0 { 0.0 } else { 2.0f32.powf(tc.clamp(-12000.0, 8000.0) / 1200.0) }
}
// -12000 / 1200 = -10 → 2^-10 ≈ 0.00098s
```

---

### 问题 3：Release 阶段指数曲线数学退化

- **问题描述**: `envelope.rs` 中声称 Decay/Release 使用 "Exponential interpolation"，但条件判断为 `if start > 0.001 && params.target > 0.0`。Release 阶段的 `target` 恒为 `0.0`，因此该条件**永远不成立**，Release 实际上退化为**线性衰减**。人耳对响度的感知是对数/指数的，线性衰减到 0 会在尾音末端产生 "突然消失" 的不自然感，进一步加剧了 "Release 音过低" 的感知问题。
- **影响文件**: `dysonphere-core/src/envelope.rs:163-170`
- **严重程度**: 🟡 High（影响音色自然度，与问题 2 叠加）
- **修复建议**: 为 Release 实现真正的指数衰减。当 target=0 时，使用指数衰减公式 `value = start * exp(-k * t)`，其中 `k` 由 `duration_samples` 推导得出，使得 `t=1.0` 时衰减到 `SILENCE_THRESHOLD`。

```rust
// envelope.rs:163-170 当前实现
Stage::Decay | Stage::Release => {
    if start > 0.001 && params.target > 0.0 {  // ← Release 时 target=0，条件永不成立
        self.value = start * (params.target / start).powf(t);
    } else {
        self.value = start + (params.target - start) * t;  // ← 退化为线性
    }
}
```

---

### 问题 4：RPN Fine Tune 实现 Bug

- **问题描述**: `synth.rs` 的 `handle_cc` 在处理 CC6（Data Entry MSB）时，RPN=0/1（Fine Tune）分支错误地使用了 `rpn_msb` 和 `rpn_lsb` 作为数据值进行计算，而不是使用传入的 `value` 参数。由于 `rpn_msb=0, rpn_lsb=1`，计算出的 `val` 恒为 `1`，导致 `fine_tune` 被设为约 `-100 cents`，完全错误。
- **影响文件**: `dysonphere-core/src/synth.rs:362-371`
- **严重程度**: 🟡 High（功能完全错误，虽然可能尚未触发）
- **修复建议**: 使用 `value` 参数（Data Entry MSB）和已存储的 Data Entry LSB（如有）组合计算。如果未实现 CC38，至少应使用 `value << 7` 或直接使用 `value` 作为 0~127 范围的值。

```rust
// synth.rs:367-371 当前实现（Bug）
} else if ch.rpn_msb == 0 && ch.rpn_lsb == 1 {
    // Fine Tune
    let val: u16 = ((ch.rpn_msb as u16) << 7) | ch.rpn_lsb as u16;  // ← 错误！应该使用 value
    let val = (val as f32 - 8192.0) / 8192.0 * 100.0;
    ch.fine_tune = val;
}
```

---

### 问题 5：Damper / Sustain 踏板未实现

- **问题描述**: `synth.rs` 的 `note_off_internal` 中，当 `damper=true` 时只有注释 "In a full implementation, track damper state per voice"，没有任何实际逻辑。这意味着 Sustain 踏板完全不起作用。
- **影响文件**: `dysonphere-core/src/synth.rs:446-456`
- **严重程度**: 🟡 High（MIDI 基本功能缺失）
- **修复建议**: 为每个 Voice 增加 `damper_sustained: bool` 字段，在 `note_off_internal` 中标记而非释放；在 `process_control` 处理 Damper Off 时，遍历并释放所有被 sustain 的 voice。

---

### 问题 6：示例代码充斥硬编码本地路径

- **问题描述**: 所有示例程序（`main.rs`, `sfz_test.rs`, `sfz_play.rs`, `vel_test.rs`, `stereo_test.rs`, `taiyang_clone.rs`, `small_blocks.rs`）都包含开发者本机的绝对路径 `/Users/jieneng/Documents/Soundfonts/...`，代码在其他机器上直接无法编译运行。此外 `taiyang_clone.rs` 和 `small_blocks.rs` 中存在大量重复代码（几乎完全相同的 `SynthEngine` 结构）。
- **影响文件**: `examples/src/*`
- **严重程度**: 🟢 Medium（工程规范问题）
- **修复建议**: 使用 `std::env::args()` 或环境变量读取路径；将 `SynthEngine` 提取到公共模块中。

---

### 问题 7：Envelope stage 索引映射脆弱

- **问题描述**: `envelope.rs` 中 `Stage` 枚举与 `stages: [StageParams; 7]` 数组之间通过手写的 `stage_index()` match 语句映射。如果未来在枚举中添加/删除阶段或调整顺序，该映射会静默出错。
- **影响文件**: `dysonphere-core/src/envelope.rs:7-15,29,238-248`
- **严重程度**: 🟢 Medium
- **修复建议**: 为 `Stage` 添加 `#[repr(u8)]` 并使用枚举的 discriminant 作为索引，或者使用 `enum_stage_count!` 宏确保编译期关联。

---

### 问题 8：缺少任何形式的自动化测试

- **问题描述**: 整个项目没有任何单元测试、集成测试或 golden master 测试。这使得音频引擎的回归测试完全依赖人工听觉判断，也是 AI 多次修改失败的根本原因之一——**没有测试能在修改后快速验证是否引入了新的削波、静音或包络异常**。
- **影响文件**: 全局
- **严重程度**: 🟡 High（严重影响可维护性）
- **修复建议**:
  1. 为 `Envelope` 编写单元测试：验证各阶段时间、曲线形状、velocity 缩放。
  2. 为 `Synthesizer` 编写集成测试：输入已知音符序列，验证输出 RMS/峰值在合理范围内。
  3. 为 SF2/SFZ 解析编写 golden test：固定测试文件，验证解析出的 `Region` 参数。

---

### 问题 9：SFZ `one_shot` 映射错误

- **问题描述**: `sfz.rs` 将 `one_shot` 映射为 `LoopMode::LoopSustain`，这完全错误。One-shot 样本应当播放一次后不循环也不受 release 影响，而 `LoopSustain` 会进入循环并在 note-off 后播放释放尾音。
- **影响文件**: `dysonphere-soundfont/src/sfz.rs:356`
- **严重程度**: 🟢 Medium（影响打击乐/鼓音色播放）
- **修复建议**: 增加 `LoopMode::OneShot` 变体，并在 `Sampler::process` 中做相应处理。

---

### 问题 10：SF2 Stereo 处理 TODO

- **问题描述**: `sf2.rs` 的 `build_stereo_samples` 直接返回单声道数据，注释 "Stereo will come later"。当前 stereo 效果仅通过 synth 层的 `pan` 实现，无法正确还原 SF2 中成对的 Left/Right 采样。
- **影响文件**: `dysonphere-soundfont/src/sf2.rs:527-540`
- **严重程度**: 🟢 Medium（功能未完工）

---

## 📋 清理优先级 (Cleanup Priority)

### 🔴 立即处理（Critical）

| 优先级 | 问题 | 预估工作量 | 阻塞风险 |
|--------|------|-----------|----------|
| 1 | **SF2 Release 默认值陷阱**（问题 2） | 0.5h | 高（AI 多次失败） |
| 2 | **增益链削波**（问题 1） | 1h | 高（影响所有输出） |
| 3 | **Release 指数曲线退化**（问题 3） | 1h | 中（与问题 2 叠加） |

### 🟡 短期优化（High Priority）

| 优先级 | 问题 | 预估工作量 |
|--------|------|-----------|
| 4 | 补充自动化测试（问题 8） | 4h |
| 5 | 修复 RPN Fine Tune Bug（问题 4） | 0.5h |
| 6 | 实现 Damper/Sustain 踏板（问题 5） | 2h |

### 🟢 长期改进（Medium/Low Priority）

| 优先级 | 问题 | 预估工作量 |
|--------|------|-----------|
| 7 | 清理示例硬编码路径与重复代码（问题 6） | 1h |
| 8 | 修复 Envelope stage 索引映射（问题 7） | 0.5h |
| 9 | 修复 SFZ one_shot 映射（问题 9） | 1h |
| 10 | 实现 SF2 真立体声（问题 10） | 4h |

---

## 🛠️ 具体清理建议（针对两大顽疾）

### 顽疾 A：默认电平过高

**当前问题代码** (`synth.rs:485-496`):
```rust
for sample in buffer.iter_mut() {
    let mut mix: f32 = 0.0;
    // ... voices loop ...
    *sample = mix;  // 无限制，mix 可能 >> 1.0
}
```

**建议重构方案**:
```rust
// 方案 1：软限制器（最小改动）
for sample in buffer.iter_mut() {
    let mut mix: f32 = 0.0;
    // ... 
    *sample = fast_tanh(mix);  // 或 mix.tanh()
}

// 方案 2：主控增益 + 过载保护（推荐）
const MASTER_GAIN: f32 = 0.25;  // 预留 4x 动态余量
for sample in buffer.iter_mut() {
    let mut mix: f32 = 0.0;
    // ...
    *sample = (mix * MASTER_GAIN).clamp(-1.0, 1.0);
}
```

**预期收益**: 消除削波失真，输出电平可预测，无需示例代码做外部归一化。

---

### 顽疾 B：默认 Release 音过低

**当前问题代码** (`sf2.rs:555` + `sf2.rs:438-440`):
```rust
fn timecents_merge(_default: i16, preset: Option<i16>, instrument: Option<i16>) -> i32 {
    i32::from(instrument.unwrap_or(-12000)) + i32::from(preset.unwrap_or(0))
}
// ...
release: timecents_to_seconds(
    timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
),
```

**建议重构方案**:
```rust
// 为 release 单独设置业务默认值（单位：秒）
const DEFAULT_RELEASE_SECONDS: f32 = 0.5;

// 方案：在 build_presets 中为 release 做最终兜底
let raw_release = timecents_to_seconds(
    timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
);
let release = if raw_release < 0.01 { DEFAULT_RELEASE_SECONDS } else { raw_release };

// 或者修改 timecents_merge 本身（仅限 release 参数）
fn timecents_merge_with_floor(preset: Option<i16>, instrument: Option<i16>, floor_tc: i16) -> i32 {
    i32::from(instrument.unwrap_or(floor_tc)) + i32::from(preset.unwrap_or(0))
}
// release: timecents_merge_with_floor(..., -6000) // -6000 tc ≈ 0.5s
```

**预期收益**: SF2 未指定 release 的音色将拥有自然尾音，AI 无需再盲目修改 `EnvelopeDescriptor::default()`。

---

## 📈 改进路线图 (Improvement Roadmap)

| 阶段 | 时间估算 | 任务内容 | 验证方式 |
|------|----------|----------|----------|
| **Day 1** | 2h | 修复 SF2 Release 默认值 + Envelope 指数衰减 + 主控增益衰减 | `vel_test` 观察峰值是否稳定；用钢琴 SF2 测试 note-off 后是否有 0.5s+ 尾音 |
| **Day 2** | 4h | 编写核心单元测试（Envelope 阶段测试、Synth 峰值测试、SF2/SFZ 解析测试） | `cargo test` 全绿 |
| **Day 3** | 2h | 修复 RPN Fine Tune + 实现 Damper 踏板 | MIDI 事件注入测试 |
| **Week 2** | 4h | 重构示例代码、提取公共 SynthEngine、清理硬编码路径 | 示例在新机器上可编译运行 |
| **后续** | 8h | SF2 真立体声、LoopMode::OneShot、性能优化 | 立体声 SF2 听感测试 |

---

## 🧠 复盘：为什么 AI 多次修改失败？

1. **只看表面，未追根溯源**: AI 可能看到 `EnvelopeDescriptor::default()` 中的 `release: 2.0`，误以为这就是 Release 时长的来源，修改了这个值。但实际上 SF2 音色走的是 `sf2.rs` 中 `timecents_merge(-12000, ...)` 的解析路径，默认值约 0.001 秒，根本不经过 `EnvelopeDescriptor::default()`。

2. **线性思维，未理解增益链**: 对于电平问题，AI 可能在某个局部环节（如 `voice.rs`）乘了一个衰减系数，但没有考虑：
   - 该系数对 SFZ 和 SF2 的副作用不同（SF2 有 `attenuation`，SFZ 没有）；
   - 多 Voice 叠加时的削波发生在 `synth.rs` 混音层，不是单个 Voice 层。

3. **没有测试闭环**: 由于项目无任何自动化测试，AI 的每次修改都无法快速验证是否有效，只能凭代码逻辑推断，导致 "改了这里、坏了那里" 的循环。

4. **缺乏领域知识**: 音频合成器的包络参数（timecents、dB、linear amp）有严格的单位换算规范。AI 可能不理解 `-12000 timecents ≈ 0.001s` 这个数量级的含义，也看不出 `max(0.15)` 对于钢琴尾音来说仍然太短。

---

*本报告基于对 dysonphere 代码库的全面静态分析生成。如需针对具体文件的逐行审计或重构方案，可进一步展开。*
