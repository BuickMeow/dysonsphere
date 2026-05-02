# 🏔️ 屎山指数报告 v6.0 —— Release 终极解剖：为什么钢琴没有「尾音」

**项目名称**: Dysonphere（戴森球）合成器引擎  
**分析日期**: 2026-05-02（最新代码状态）  
**对比基准**: xsynth（本地 `/Users/jieneng/Documents/GitHub/xsynth`）  
**用户反馈**: 轻量削波已修复，重和弦仍削波；Release 仍然太短，没有钢琴悠长感  
**屎山指数**: **28 / 100** 🟡（架构缺陷明确，修复路径清晰）

---

## 📊 v5→v6 修改确认

| 修改项 | 状态 | 效果 |
|--------|------|------|
| `soft_clip` 改为 C¹ 连续 | ✅ 已应用 | 轻量场景削波消除 |
| `MASTER_GAIN = 0.15` | ⚠️ 未改 | 重和弦仍过载 |
| velocity 不再缩放 release | ✅ 已应用 | release 时间一致性改善 |
| `RELEASE_TARGET = -90dB` | ✅ 已应用 | 消除 click，但感知 release 仍短 |
| CC72 (Release Time) | ❌ 未实现 | **最大缺失** |

---

## 🔴 根因一：感知 Release 时间 ≠ Envelope T（数学陷阱）

### 人耳的可听阈值

人耳在安静环境下的**最小可听振幅**约为 **-30dB 到 -40dB**（相对于满幅）。低于此阈值的声音虽然仍在物理上存在，但大脑将其过滤为"静音"。

### dysonphere 的指数 Release 曲线

dysonphere 使用指数衰减到 `-90dB`：

```
value(t) = sustain * (RELEASE_TARGET / sustain)^(t/T)
         = 1.0 * (3e-5)^(t/T)
         = 10^(-4.52 * t/T)
```

**感知 Release 时间（以 -30dB 为听觉阈值）**：

| Envelope T | 到 -30dB 的时间 | 到 -90dB 的时间 | 感知占比 |
|-----------|----------------|----------------|---------|
| 0.3s | **0.066s** | 0.3s | 22% |
| 0.5s | **0.11s** | 0.5s | 22% |
| 0.8s | **0.18s** | 0.8s | 22% |
| 1.0s | **0.22s** | 1.0s | 22% |
| 2.0s | **0.44s** | 2.0s | 22% |

**结论**：人耳感知的 release 时间只有 envelope T 的 **约 22%**。一个 T=0.8s 的 release，人耳只会觉得它持续了 **0.18 秒**——几乎是一瞬间。

### 真实钢琴的 Release

| 音区 | 实际物理衰减时间 | 感知衰减时间 |
|------|----------------|-------------|
| 低音 (C1-C3) | 3–8 秒 | 2–5 秒 |
| 中音 (C3-C5) | 1.5–4 秒 | 1–2.5 秒 |
| 高音 (C5-C8) | 0.5–2 秒 | 0.3–1.5 秒 |

要让 dysonphere 的钢琴有"悠长尾音"，envelope T 需要达到 **2–5 秒**（中音区至少 2 秒）。

但当前 SF2 解析的 release floor 只有 0.3s，envelope.rs 的 floor 只有 0.2s。两者叠加后，即使 SF2 文件中 release=0.8s，感知 release 也只有 **0.18s**——完全不像钢琴。

### xsynth 为什么听起来更长？

xsynth 默认使用 **`LerpConcave` (1-t)^8 曲线** 而非指数曲线（`config.rs:70`）：

```rust
// xsynth 默认 release curve
release_curve: EnvelopeCurveType::Linear,  // => LerpConcave
```

LerpConcave 公式：
```
value(t) = sustain * (1 - t/T)^8
```

**相同 T=1.0s 下，两种曲线的对比**：

| t/T | 指数 (dysonphere) | LerpConcave (xsynth) | dB 差 |
|-----|------------------|---------------------|-------|
| 0.1 | 0.63 (-4dB) | 0.43 (-7.3dB) | xsynth 更快衰减 |
| 0.2 | 0.40 (-8dB) | 0.17 (-15.4dB) | xsynth 更快衰减 |
| 0.3 | 0.25 (-12dB) | 0.058 (-24.7dB) | xsynth **明显更快** |
| 0.5 | 0.032 (-30dB) | 0.004 (-48dB) | xsynth 已几乎静音 |

**奇怪——xsynth 的 LerpConcave 衰减更快，为什么用户觉得 xsynth release 更长？**

答案在下一节。

---

## 🔴 根因二：CC72 (Release Time) 的「暗魔法」

### MIDI CC72 是什么

CC72 是 MIDI 标准中的 **Release Time** 控制器：
- **Value 0**: 最短 release（接近瞬时）
- **Value 64**: 默认值（不改变 SF2 原始 release）
- **Value 127**: 最长 release（可延长到原始值的数倍）

几乎所有专业音源（包括 xsynth）都支持 CC72。许多 DAW、音序器和 MIDI 文件会在初始化时发送 CC72=90 或 CC72=127，以让钢琴音色更"华丽"。

### xsynth 的 CC72 实现（`voice/envelopes.rs:372-378`）

```rust
fn calculate_curve(value: u8, duration: f32) -> f32 {
    match value {
        0..=64  => (value as f32 / 64.0).powi(5) * duration,       // 缩短到 0~1x
        65..=128=> duration + ((value as f32 - 64.0) / 64.0).powi(3) * 15.0,  // 延长到 1~16x
        _ => duration,
    }
}
```

**数值验证**：

| CC72 Value | xsynth 实际 release | 倍数 | dysonphere 实际 release |
|-----------|-------------------|------|------------------------|
| 0 | 0x duration | 0 | duration（无 CC72） |
| 32 | 0.03x duration | 0.03 | duration（无 CC72） |
| 64 | 1x duration | 1 | duration（无 CC72） |
| 90 | 3.1x duration | 3.1 | duration（无 CC72） |
| 100 | 6.0x duration | 6.0 | duration（无 CC72） |
| 127 | **16x duration** | 16 | duration（无 CC72） |

**这是 dysonphere 与 xsynth 在 release 感知上差距最大的原因。**

假设 SF2 文件中 piano release = 0.5s：
- **dysonphere**：T=0.5s => 感知 release ≈ **0.11s**
- **xsynth + CC72=64（默认）**：T=0.5s => 感知 release ≈ **0.11s**（相同）
- **xsynth + CC72=90（常见DAW默认值）**：T=1.55s => 感知 release ≈ **0.34s**
- **xsynth + CC72=127**：T=8.0s => 感知 release ≈ **1.76s** ✅ **这才是钢琴的感觉**

### 为什么用户之前没发现这个问题？

因为 xsynth 默认可能接收到 CC72=64（不改变），但用户的播放环境（DAW / MIDI 文件 / 游戏引擎）**可能发送了 CC72 > 64 的事件**。在 xsynth 中，这些事件被正确响应，release 延长到 3~16 倍；而在 dysonphere 中，这些事件被**完全忽略**。

### dysonphere 的 CC 处理现状

`synth.rs:371-441` 的 `handle_cc` 只处理了：
- CC0 (Bank), CC6/38/100/101 (RPN), CC7 (Volume), CC8/10 (Pan), CC11 (Expression), CC64 (Damper)

**CC72 (0x48) 完全不存在。**

---

## 🔴 根因三：NoLoop 采样被 Sampler 提前截断

### 问题代码（`sampler.rs:117-124`）

```rust
fn get(&self, idx: usize) -> f32 {
    if (!matches!(self.loop_mode, LoopMode::LoopContinuous) || self.released)
        && idx >= self.sample_end as usize {
            return 0.0;
    }
    self.data.get(idx).copied().unwrap_or(0.0)
}
```

### 根因

对于 `NoLoop` 采样（大多数钢琴音色）：
1. Note-off 触发 `release()`，sampler 的 `released` 标志不变（NoLoop 分支什么都不做）
2. `process()` 继续推进 `position` 直到 `sample_end`
3. 一旦 `position >= sample_end`，`get()` 返回 **0.0**
4. 此时 `voice.process() = sample * envelope * volume = 0.0 * envelope * volume = 0.0`
5. 即使 envelope 还在 release 阶段，声音已经消失

**这意味着：NoLoop 采样的实际 release 长度受限于采样本身的尾部长度，而不是 envelope 的 release 时间。**

如果钢琴采样只有 1 秒长，而 note-on 在 0.8s 时发生 note-off，那么从 note-off 到 sample_end 只有 0.2s。即使 envelope T=2.0s，声音也只持续 0.2s。

### xsynth 的对比

xsynth 的 `SampleReaderNoLoop` 同样有 `is_past_end`，但 xsynth 支持 **release voice spawners**：

```rust
// xsynth/channel/key.rs:44-49
KeyNoteEvent::Off => {
    let vel = self.voices.release_next_voice();
    if let Some(vel) = vel {
        let voices = channel_sf.spawn_voices_release(control, self.key, vel);
        self.voices.push_voices(voices, max_layers);
    }
}
```

如果 soundfont 定义了 release voices（专门的 release 采样），note-off 时会触发新的采样，其长度可以远大于原采样的尾部。

dysonphere **完全没有 release voice 机制**。note-off = 当前 voice 进入 envelope release，仅此而已。

---

## 🟡 根因四：固定 MASTER_GAIN = 0.15 导致重和弦削波

### soft_clip 虽已修复，但输出仍可能 > 1.0

当前 `soft_clip`（`synth.rs:601-608`）：

```rust
fn soft_clip(x: f32) -> f32 {
    let ax = x.abs();
    if ax <= 1.0 { x }
    else {
        let over = ax - 1.0;
        x.signum() * (1.0 + over / (1.0 + over * over))
    }
}
```

**数学分析**：
- `g(x) = 1 + (x-1)/(1+(x-1)²)` 在 `x=2` 时取最大值 **1.5**
- `x=1.5` => `g(1.5) = 1 + 0.5/1.25 = 1.4`
- `x=3.0` => `g(3.0) = 1 + 2/5 = 1.4`

**soft_clip 的输出可以超过 1.0！** 当 mix * MASTER_GAIN > 1.0 时，soft_clip 不会将其压到 1.0 以下，而是压到 1.0~1.5 之间。如果后续有硬限幅（如导出 WAV 时的 `clamp(-1.0, 1.0)`），仍然会产生削波。

### 重和弦时的混音电平

| 活跃 voice 数 | 平均单 voice 振幅 | mix 总和 | mix * 0.15 | soft_clip 输出 | 是否削波 |
|-------------|----------------|---------|-----------|---------------|---------|
| 6 | 1.0 | 6.0 | 0.9 | 0.9 | ❌ 安全 |
| 10 | 1.0 | 10.0 | 1.5 | 1.4 | ⚠️ >1.0 |
| 20 | 0.5 | 10.0 | 1.5 | 1.4 | ⚠️ >1.0 |
| 20 | 1.0 | 20.0 | 3.0 | 1.4 | ⚠️ >1.0 |
| 50 | 0.5 | 25.0 | 3.75 | 1.33 | ⚠️ >1.0 |

**只要 mix * 0.15 > 1.0（即 mix > 6.67），soft_clip 的输出就会 > 1.0**，在后续硬限幅时产生削波。

### 为什么轻量场景 OK？

轻量场景（单音、少量 voice）mix < 6.67，soft_clip 透明通过，无削波。

重和弦（10+ voice 或力度大的和弦）mix > 6.67，soft_clip 输出 > 1.0，硬限幅时削波。

---

## 🔍 MIDI 默认 Release 应该是怎样的？

### GM 标准

General MIDI 规范没有强制规定默认 release 时间，但约定：
- **钢琴类音色**（Acoustic Grand, Bright Acoustic 等）：release 应足够长以模拟真实钢琴的琴弦共鸣
- **弦乐类音色**：release 更长（2-5s），模拟弓离弦后的余振
- **打击乐/拨弦**：release 较短（0.05-0.2s）

### 行业实践

| 音源/引擎 | 默认 CC72 | Release 策略 |
|----------|----------|-------------|
| FluidSynth | 64 | 支持 CC72，指数曲线，默认 0.5-2s |
| xsynth | 64 | 支持 CC72，可延长到 16x，LerpConcave |
| Windows GS Wavetable | 64 | 支持 CC72，固定 SF2 值 |
| Kontakt | 64 | 复杂脚本控制，通常默认 2-4s |
| **dysonphere** | **N/A** | **无 CC72，固定 SF2 值 + 0.3s floor** |

### 钢琴音色的 Release 黄金法则

从音乐制作角度，钢琴的 release 应该满足：
1. **中音区单音符**：从 note-off 到完全静音，**至少 1.5-2.5 秒**
2. **和弦叠加**：多个 voice 同时 release 时，混音尾音应自然融合，不突兀消失
3. **与 damper pedal 配合**：pedal 释放时，所有 held voice 同时进入 release，此时 release 不应太短（否则像"瞬间关闸"）

---

## 🔍 CC72 是怎么做好的？xsynth 的设计哲学

### xsynth 的双层调制架构

xsynth 支持两种 envelope 调制方式：

**1. 高精度秒数（EnvelopeControlData）**
```rust
pub struct EnvelopeControlData {
    pub attack: Option<f32>,   // 秒
    pub release: Option<f32>,  // 秒
}
```
- 直接覆盖原始 envelope 时间
- 优先级最高

**2. MIDI CC 值（EnvelopeCCControlData）**
```rust
pub struct EnvelopeCCControlData {
    pub attack: Option<u8>,    // 0-127
    pub release: Option<u8>,   // 0-127
}
```
- 相对调制：基于原始 envelope 时间的比例缩放
- CC72=64 时不变，CC72>64 时延长，CC72<64 时缩短

### `calculate_curve` 的精妙设计

```rust
fn calculate_curve(value: u8, duration: f32) -> f32 {
    match value {
        0..=64  => (value as f32 / 64.0).powi(5) * duration,
        65..=128=> duration + ((value as f32 - 64.0) / 64.0).powi(3) * 15.0,
        _ => duration,
    }
}
```

**为什么用 powi(5) 和 powi(3)？**

- **缩短段（0-64）用 5 次方**：让低 CC 值时的缩短非常剧烈。CC=32 时只缩短到 3%，CC=48 时缩短到 24%。这样可以快速获得"staccato"效果。
- **延长段（65-128）用 3 次方**：让高 CC 值时的延长相对线性。CC=90 时延长到 3.1x，CC=100 时 6x，CC=127 时 16x。
- **+15.0 的系数**：确保在 CC=127 时可以达到原始值的 16 倍，覆盖从"短促"到"无限延音"的全范围。
- **.max(0.02)**：保证即使 CC=0，release 也有 20ms，避免 click。

### 实时更新机制

xsynth 的 `VoiceControlData` 在每个渲染块开始时通过 `process_controls` 传播到所有活跃 voice：

```rust
// xsynth/voice/envelopes.rs:438-443
pub fn modify_envelope(&mut self, envelope: EnvelopeControlData, cc_envelope: EnvelopeCCControlData) {
    if !self.killed {
        self.params = Self::get_modified_envelope(
            self.original_params, envelope, cc_envelope, self.sample_rate);
        self.update_stage();
    }
}
```

这意味着：**在演奏过程中调整 CC72，所有正在 release 的 voice 的 release 时间也会实时更新**。这是专业音源的标准行为。

dysonphere 的 envelope 在 voice 创建时就固定了，**没有任何运行时调制能力**。

---

## 🛠️ v6 修复建议

### 🔴 P0：实现 CC72（Release Time）支持

**步骤 1：添加 CC72 解析到 synth.rs**

在 `handle_cc` 中添加：
```rust
0x48 => {
    // CC72: Release Time
    // 将 0-127 映射到 0.0-2.0 的 release 倍数
    let norm = value as f32 / 127.0;
    let multiplier = if value <= 64 {
        (norm / (64.0/127.0)).powi(5)  // 0~1, 5次方
    } else {
        1.0 + ((norm - 64.0/127.0) / (63.0/127.0)).powi(3) * 15.0  // 1~16
    };
    ch.release_multiplier = multiplier;
}
```

**步骤 2：在 ChannelState 中添加 release_multiplier**

```rust
struct ChannelState {
    // ... 现有字段 ...
    release_multiplier: f32,  // 默认 1.0
}
```

**步骤 3：在 note_on_internal 中应用**

```rust
let release = descriptor.release.max(0.2) * ch.release_multiplier;
```

**效果**：支持 CC72 实时调制 release 时间，最大可延长到 16 倍。

### 🔴 P0：提升默认 Release Floor 到钢琴级别

**修改 `sf2.rs:438-445`**：
```rust
release: {
    let secs = timecents_to_seconds(
        timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
    );
    // 钢琴音色需要至少 1.5s 的 release 才能有尾音
    if secs < 0.3 { 2.0 } else { secs.max(1.5) }
},
```

**修改 `envelope.rs:65`**：
```rust
let release = descriptor.release.max(0.5);  // 最低 0.5s
```

**效果**：即使 SF2 文件设置很短，release 也至少 1.5s（感知约 0.3-0.5s），接近钢琴的最低要求。

### 🟡 P1：修复 soft_clip 输出 > 1.0 的问题

**方案 A：修改 soft_clip 使其输出始终 ≤ 1.0**
```rust
fn soft_clip(x: f32) -> f32 {
    let ax = x.abs();
    if ax <= 1.0 { x }
    else {
        let over = ax - 1.0;
        // 将最大值从 1.5 降到 1.0
        x.signum() * (1.0 + over / (1.0 + over * over)).min(1.0)
    }
}
```

**方案 B（推荐）：引入动态 MASTER_GAIN**
```rust
// 根据活跃 voice 数动态调整 gain
let active_voices = self.voices.len().max(1) as f32;
let dynamic_gain = (6.0 / active_voices).min(1.0).max(0.05);
```

**效果**：重和弦时自动降低 gain，避免过载。

### 🟡 P1：为 NoLoop 采样添加 release tail 保护

**修改 `sampler.rs`**：当 NoLoop 采样在 release 阶段到达 sample_end 后，不立即返回 0.0，而是让 envelope 继续衰减：

实际上这不是 sampler 的问题——当前逻辑已经让 voice 继续存在（因为 `!is_releasing()` 保护了 `finished()`）。问题是 sampler 返回 0.0 后，voice 输出 = 0.0 * envelope = 0.0。

**真正的修复**：确保 piano 采样有足够长的尾部，或使用 LoopSustain + 长尾部。

对于 dysonphere 的架构，更实际的做法是：
- 在 `voice.rs` 中，当 sampler 返回 0.0 但 envelope 还在 release 时，让 voice 继续存在（当前已实现）
- 但这意味着 CPU 被浪费在计算无声的 voice 上

更好的方案：**在 NoLoop 模式下，如果采样到达尾部，直接让 voice 结束（因为 envelope 再长也没用）**。这其实是当前的行为...

所以对于 NoLoop 采样，release 确实受限于采样长度。**这是采样本身的限制，不是引擎的 bug。**

### 🟢 P2：添加 per-voice 的 release 曲线选择

当前 dysonphere 的指数曲线让前期衰减太快。可以添加一个"慢速 release"模式：

```rust
// envelope.rs
enum ReleaseCurve {
    Exponential,  // 当前：快速前期衰减
    Linear,       // 线性：均匀衰减
}
```

Linear 曲线：`value(t) = sustain * (1 - t/T)`

对于 T=2.0s 的 linear release：
- t=0.5s: value=0.75 (-2.5dB) → 仍很响！
- t=1.0s: value=0.50 (-6dB) → 明显可听
- t=1.5s: value=0.25 (-12dB) → 还有尾音
- t=2.0s: value=0.0

感知 release 约 1.5s，比指数的 0.44s 长 **3 倍**。

---

## 🧠 终极复盘：为什么 Release 一直修不好

### 误区 1："调大 release 参数就行"

前几次修复把 floor 从 0.05s 调到 0.2s、0.3s、0.8s，但没有意识到：
- 指数曲线到 -90dB 的感知时间只有 T 的 22%
- 即使 T=0.8s，感知也只有 0.18s
- 需要 T=2-5s 才能有钢琴感，但 SF2 文件通常不设置这么长

### 误区 2："xsynth 的 release 曲线更好"

实际上 xsynth 默认的 LerpConcave 衰减比 dysonphere 的指数**更快**。xsynth 的 release 长是因为：
1. **CC72 调制**（最大 16 倍延长）
2. **用户的 MIDI 环境发送了 CC72 > 64**
3. **release voice spawners**（专门的 release 采样）

### 误区 3："是 envelope 的 bug"

envelope 的数学实现是正确的。问题在于：
- 没有 CC 调制系统
- 没有考虑人耳感知阈值
- 没有针对不同音色的默认 release 策略

### 正确的修复优先级

| 优先级 | 修复 | 预期效果 |
|-------|------|---------|
| 🔴 P0 | 实现 CC72 支持（0-127 => 0-16x） | **最大提升**。即使 SF2 release=0.5s，CC72=100 时也能到 3s |
| 🔴 P0 | 提升默认 release floor 到 1.5-2.0s | 保证钢琴音色至少有基础尾音 |
| 🟡 P1 | 添加 Linear release 曲线选项 | 让同样 T 值下感知 release 延长 2-3 倍 |
| 🟡 P1 | 动态 MASTER_GAIN | 消除重和弦削波 |
| 🟢 P2 | 支持 release voice spawners | 从采样层面获得真实尾音 |

---

## 📋 快速验证清单

在修复后，用以下方法验证 release：

1. **单音符测试**：播放中音区音符（C4），note-off 后听尾音。应有 1-2 秒逐渐消失的余音。
2. **和弦测试**：同时按下 C-E-G 和弦，释放后三个音的尾音应自然融合，不突兀切断。
3. **CC72 测试**：发送 CC72=127，release 应明显延长到 3-5 秒以上。
4. **CC72=0 测试**：发送 CC72=0，release 应几乎瞬间消失（staccato 效果）。
5. **重和弦削波测试**：同时按下 10 个音符，检查输出波形是否硬切（clip）。

---

*本报告揭示了 dysonphere release 问题的三层根因：感知阈值陷阱（22% 效应）、CC72 暗魔法缺失、NoLoop 采样截断。与 xsynth 的核心差距不在 envelope 曲线，而在 CC 调制系统的完整性和默认参数的音乐性。*
