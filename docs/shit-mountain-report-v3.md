# 🏔️ 屎山指数报告 v3.0 —— 爆音根因终极诊断版

**项目名称**: Dysonphere（戴森球）合成器引擎  
**分析日期**: 2026-05-02  
**对比基准**: xsynth（本地成熟项目）  
**核心结论**: **soft_clip 是数学上完全错误的"限幅器"**，它在输入超过 ±3.0 时输出反而增大，导致 "限幅" 后的信号比限幅前更大，外部 normalize 把这些虚假峰值当作 max 缩放，把整个文件的动态范围压扁。这是爆音的根本原因。

---

## 📊 Overall Assessment

- **屎山指数**: **55/100**（中高等级，较 v2 改善有限，因为新增代码引入了新的错误）
- **爆音严重程度**: 🔴 **Critical**（用户反馈"仍然十分严重，感觉和原来没什么区别"）
- **新增问题**: soft_clip 数学错误、normalize 放大不连续、事件时序设计缺陷
- **风险等级**: 🔴 **Critical**

---

## 🎯 核心发现：soft_clip 是一个"反向放大器"

### 错误代码（`synth.rs:585-588`）

```rust
#[inline]
fn soft_clip(x: f32) -> f32 {
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}
```

### 数值验证

| 输入 x | soft_clip(x) | 应该趋近于 | 实际行为 |
|--------|-------------|-----------|---------|
| 1.0 | 0.778 | ~0.762 (tanh) | ✅ 近似正确 |
| 2.0 | 0.984 | ~0.964 | ✅ 近似正确 |
| **3.0** | **1.000** | ~0.995 | ⚠️ 刚好到1.0 |
| **4.0** | **1.006** | ~0.999 | 🔴 **超过1.0！** |
| **5.0** | **1.032** | ~0.9999 | 🔴 **继续增大！** |
| **10.0** | **1.370** | ~1.0000 | 🔴 **放大到1.37倍！** |
| 100.0 | 11.05 | ~1.0000 | 🔴 **放大到11倍！** |

**结论**: 这个公式根本不是限幅器。它是一个在 `[-3, 3]` 范围内近似 tanh 的函数，但在 `|x| > 3` 时，输出反而趋向于 `x / 9`（线性增长，无上界）。

### 为什么用户觉得"没什么区别"

**场景**: 弹一个三和弦，3 个 note，每个 note 可能有 1-2 个 region 叠加，总 mix ≈ 3.0~6.0。

1. soft_clip(3.0) = 1.0，看似正常
2. soft_clip(6.0) = 6 * 63 / 351 = **1.077**（超过1.0！）
3. 外部 normalize 发现 max = 1.077，把所有 sample 乘以 `0.9 / 1.077 = 0.835`
4. 原来 mix = 1.0 的 sample 被压缩到 0.835
5. 原来 mix = 6.0 的 sample 被压缩到 0.9
6. **动态范围被压扁，听起来扁平、失真**

更糟的是：如果某个瞬间有 voice steal 或 attack transient 导致 mix = 20.0：
- soft_clip(20) = 20 * 427 / 3627 = **2.35**
- normalize: `0.9 / 2.35 = 0.383`
- 正常 sample 被压缩到原来的 38%！
- 从 0.383 跳到 0.9 的 sample 产生剧烈不连续 → **爆音**

---

## 🔬 xsynth 是怎么做的？—— 多层级 Gain Staging

### 层级 1：SF2 Modulator 衰减（单 voice 源头控制）

xsynth 在 `sf2/modulator.rs:706-711` 实现了 SF2 规范**强制要求的默认 modulators**：

```rust
pub(crate) fn default_note_modulators() -> [Sf2NoteModulator; 2] {
    [
        Sf2NoteModulator::default_velocity_to_attenuation(),  // vel=1 时衰减 ~-96dB
        Sf2NoteModulator::default_velocity_to_filter_cutoff(),
    ]
}
```

这意味着 xsynth 的单个 voice 在 low velocity 时**自动衰减到几乎静音**，而 dysonphere 的 voice 仍然接近满幅。

### 层级 2：Volume 二次曲线（Channel 控制）

xsynth 在 `channel/mod.rs:159-164`：

```rust
for sample in out.iter_mut() {
    let vol = control.volume.get_next() * control.expression.get_next();
    let vol = vol.powi(2);  // <-- 平方！
    *sample *= vol;
}
```

Volume 控制是**二次的**。即使 volume=0.5，实际衰减是 0.25（-12dB），而不是线性的 0.5（-6dB）。这为人耳提供了更自然的响度感知。

### 层级 3：Voice Layer Limit（并发控制）

xsynth 在 `channel/params.rs:53`：

```rust
pub layers: Option<usize>,  // 默认 Some(4)
```

每个 key 最多同时激活 4 个 voice layer。dysonphere 没有 layer limit，单个 note 可能触发十几个 region。

### 层级 4：VolumeLimiter（最终防护）

xsynth 在 `effects/limiter.rs` 实现了真正的自适应限幅器：

```rust
fn limit(&mut self, val: f32) -> f32 {
    let abs = val.abs();
    // 跟踪 loudness 历史（attack=100 samples, falloff=16000 samples）
    if self.loudness > abs {
        self.loudness = (self.loudness * self.falloff + abs) / (self.falloff + 1.0);
    } else {
        self.loudness = (self.loudness * self.attack + abs) / (self.attack + 1.0);
    }
    // 自适应衰减：loudness 越大，衰减越多
    val / (self.loudness * self.strength + 2.0 * (1.0 - self.strength)) / 2.0
}
```

但注意：xsynth 的 limiter 在 `ChannelGroup` 级别**默认不启用**（需要显式配置）。xsynth 主要依靠前 3 层的 gain staging 来避免削波。

### dysonphere 缺少的层级

| 层级 | xsynth | dysonphere | 影响 |
|------|--------|-----------|------|
| Velocity-to-Attenuation | ✅ 默认 modulator | ❌ 无 | 低 velocity 过响 |
| Volume 曲线 | ✅ 二次 (powi(2)) | ❌ 线性 | 响度控制不自然 |
| Layer Limit | ✅ 默认 4 | ❌ 无限制 | 多 region 叠加削波 |
| Master Gain | ✅ Limiter 兜底 | ❌ 无 / 错误 soft_clip | 最终输出失控 |

---

## 🔍 其他爆音来源分析

### 来源 2：外部 Normalize 的动态毁灭

**错误代码**（`examples/src/main.rs:87-94`）：

```rust
// Normalize
let max = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
if max > 0.0 {
    let gain = 0.9 / max;
    for s in &mut buffer {
        *s *= gain;
    }
}
```

**问题**：
1. normalize 是**全局的**，基于整个 buffer 的 max
2. 如果 buffer 中某个瞬间有异常峰值（如 mix=20.0），normalize 会把所有正常 sample 压缩到原来的 4.5%
3. 这导致**整体音量过低 + 动态范围被压扁**
4. 更严重的是：如果异常峰值是由错误（如 soft_clip 的 NaN 或 Inf）引起的，normalize 会把 NaN/Inf 传播到整个 buffer

**验证 soft_clip 的数值稳定性**：
- 当 mix = 1.84e19 时，x² 溢出为 Inf
- soft_clip(Inf) = Inf * Inf / Inf = **NaN**
- NaN 经过 normalize 后，整个 buffer 变成 NaN

虽然正常 mix 不会达到 1.84e19，但如果某个 voice 的 volume 异常大（如 1e10），就可能触发。

### 来源 3：Voice Stealing 无 Fade Out

**当前代码**（`synth.rs:501-513`）：

```rust
fn steal_voice(&mut self) {
    if let Some(idx) = self.voices.iter().position(|(_, v)| v.is_releasing()) {
        self.voices.swap_remove(idx);
        return;
    }
    self.voices.remove(0);  // <-- 直接移除最老的 voice，无 fade out！
}
```

xsynth 的处理（`voice_buffer.rs:59-101`）：
- 优先移除 velocity 最低的 voice（最安静的）
- 支持 `fade_out_killing: true` 模式，给被移除的 voice 一个 1ms 的 kill release
- 即使默认 `fade_out_killing: false`，也是基于 velocity 选择移除目标，减少可感知性

dysonphere 直接移除最老的 voice。如果这个 voice 当时正在输出非零 sample（如 sustain 阶段的峰值），移除后 mix 值会突变。

### 来源 4：事件时序导致的 Release 重叠缺失

**用户示例代码**（`main.rs:76-81`）：

```rust
for (i, &note) in notes.iter().enumerate() {
    synth.note_on(note, 100);
    let offset = i * note_samples;
    synth.read_samples(&mut buffer[offset..offset + note_samples]);
    synth.note_off(note);  // <-- NoteOff 在 read_samples 之后！
}
```

**问题**：
- NoteOff 在 `read_samples` 之后发送，事件进入 pending
- 下一个 `read_samples` 开始时 `flush_events()` 处理 NoteOff
- 但下一个 note 的 NoteOn 也在下一个 `read_samples` 之前发送
- 这意味着**前一个 note 的 release 和后一个 note 的 attack 发生在同一个 buffer 中**
- 如果 release 时间短（如 0.05s），而 note 间隔是 0.5s，前一个 note 的 release 只在前 0.05s 的 buffer 中渲染，后 0.45s 已经静音
- 但问题是：如果用户测试时听的是单个 note，他需要在 NoteOff 后**额外 read_samples** 才能听到 release tail

在 `main.rs` 中：
```rust
synth.note_off(note);
// 没有 read_samples 来渲染 release tail！
```

NoteOff 后没有立即 read_samples，release tail 被延迟到下一个 note 的 buffer 中。但由于下一个 note 的 NoteOn 也在那个 buffer 之前发送，release tail 和新的 attack 混合在一起。

如果 release 很短，release tail 被新 note 的 attack 掩盖，用户根本听不到 release。

**但这不是爆音的来源，只是 release 听不清的原因。**

---

## 🛠️ 修复方案（按优先级排序）

### 🔴 立即修复 1：替换 soft_clip 为真正的限幅器

**当前（错误）**：
```rust
fn soft_clip(x: f32) -> f32 {
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)  // 在 |x|>3 时反向放大！
}
```

**修复方案 A（简单有效）**：
```rust
/// 真正的软限幅：tanh 近似，保证 |output| <= 1.0
#[inline]
fn soft_clip(x: f32) -> f32 {
    // 先限制输入范围，防止近似公式失效
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}
```

**修复方案 B（更精确）**：
```rust
#[inline]
fn soft_clip(x: f32) -> f32 {
    // 分段近似 tanh，保证 |output| < 1.0
    let ax = x.abs();
    let y = if ax < 1.0 {
        x * (1.0 + ax * (-0.333333 + ax * 0.133333))
    } else if ax < 2.0 {
        let t = ax - 1.0;
        x.signum() * (0.8 + t * (0.2 - t * 0.066667))
    } else {
        // ax in [2, 3]
        let t = ax - 2.0;
        x.signum() * (0.933333 + t * (0.066667 - t * 0.016667))
    };
    y.clamp(-1.0, 1.0)
}
```

**修复方案 C（最简单，推荐作为第一步）**：
```rust
/// 硬截断 + 轻微压缩：在 |x|<=1 时线性，|x|>1 时压缩
#[inline]
fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 1.0 {
        x
    } else {
        x.signum() * (1.0 - 0.5 / x.abs())
    }
}
```

方案 C 的特点是：
- |x| ≤ 1：完全透明，无失真
- |x| = 2：输出 ±0.75
- |x| = 10：输出 ±0.95
- 永远单调趋向于 ±1.0

### 🔴 立即修复 2：增加 MASTER_GAIN 预留动态余量

**在 `synth.rs` 中**：

```rust
/// 主控增益：为混音预留动态余量。
/// 假设最多 8 个 voice 同时满幅叠加，1/8 = 0.125，取 0.15 提供约 -16dB 余量。
const MASTER_GAIN: f32 = 0.15;

// Mono 路径
*sample = soft_clip(mix * MASTER_GAIN);

// Stereo 路径
chunk[0] = soft_clip(left * MASTER_GAIN);
chunk[1] = soft_clip(right * MASTER_GAIN);
```

这样即使 10 个 voice 叠加（mix = 10.0），输入到 soft_clip 的值是 1.5，输出约 0.83，不会削波。

### 🟡 短期修复 3：移除外部 Normalize，改用固定增益

**在 `examples/src/main.rs` 中**：

```rust
// 删除这段毁灭动态的代码：
// let max = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
// if max > 0.0 {
//     let gain = 0.9 / max;
//     for s in &mut buffer { *s *= gain; }
// }

// 替换为固定增益（如果 MASTER_GAIN=0.15，这里不需要额外增益）
// 或者如果 MASTER_GAIN 未设置，使用保守的固定增益：
const OUTPUT_GAIN: f32 = 0.3;
for s in &mut buffer {
    *s = (*s * OUTPUT_GAIN).clamp(-1.0, 1.0);
}
```

### 🟡 短期修复 4：实现 SF2 Velocity-to-Attenuation（最小版本）

**在 `sf2.rs` 的 `build_presets` 中**，为每个 region 的 volume 增加 velocity 衰减：

```rust
// 最小实现：concave 曲线的近似
fn vel_to_attenuation(vel: u8) -> f32 {
    let norm = vel as f32 / 127.0;
    // SF2 默认 modulator：concave 曲线，近似为平方根
    norm.sqrt()
}

// 在 build_presets 中，为 volume 乘上 velocity 衰减
let volume = 10.0f32.powf(-attenuation as f32 / 200.0) * vel_to_attenuation(vel);
```

**注意**：这需要在 `Region` 或 `VoiceParams` 中存储 velocity 相关的信息，或者直接在解析时应用。

更简单的方案是在 `Voice::new` 中修改：

```rust
// voice.rs
let vel_norm = velocity as f32 / 127.0;
let vel_amp = vel_norm.powi(2);  // 现有的二次曲线
// 增加 SF2 风格的 concave 衰减
let vel_atten = vel_norm.sqrt();
let volume = params.volume * vel_amp * vel_atten;
```

但这会双重衰减。更好的方案是：

```rust
// 用 concave 替代原来的平方
let vel_amp = (velocity as f32 / 127.0).sqrt();
```

### 🟡 短期修复 5：Voice Stealing 增加 Kill Release

```rust
fn steal_voice(&mut self) {
    // 优先 steal releasing voice
    if let Some(idx) = self.voices.iter().position(|(_, v)| v.is_releasing()) {
        self.voices.swap_remove(idx);
        return;
    }
    // 否则给最老的 voice 一个 5ms 的 kill release，而不是直接移除
    if let Some((_, voice)) = self.voices.first_mut() {
        voice.kill();  // kill() 已经设置了短 release
        // 不要立即移除，让它在下一帧自然结束
    }
}
```

但 `steal_voice` 在 `note_on_internal` 中调用，需要立即为新 voice 腾出 slot。所以更好的方案是：

```rust
fn steal_voice(&mut self) {
    if let Some(idx) = self.voices.iter().position(|(_, v)| v.is_releasing()) {
        self.voices.swap_remove(idx);
        return;
    }
    // 移除最安静的 voice（velocity 最低），而不是最老的
    if let Some((quietest_idx, _)) = self.voices.iter()
        .enumerate()
        .min_by_key(|(_, (_, v))| v.velocity) {
        self.voices.swap_remove(quietest_idx);
    }
}
```

### 🟢 长期改进 6：事件时序修正

**用户示例应该改为**：

```rust
// 渲染 NoteOff 的 release tail
synth.note_off(note);
synth.read_samples(&mut release_buffer);  // 单独的 buffer 渲染 release
```

或者在 dysonphere 的 `Synthesizer` 中增加一个 `flush` 方法：

```rust
/// 处理所有 pending 事件但不渲染音频
pub fn flush(&mut self) {
    self.flush_events();
}
```

这样用户可以：
```rust
synth.note_off(note);
synth.flush();  // 立即处理 NoteOff
// 然后 read_samples 渲染 release tail
```

---

## 📋 清理优先级（v3 最终版）

### 🔴 立即处理（Critical — 今天必须改）

| # | 问题 | 文件 | 修改内容 | 验证方式 |
|---|------|------|---------|----------|
| 1 | **soft_clip 反向放大** | `synth.rs` | 替换为 `x.clamp(-3,3)` + 原公式，或方案 C | 计算 soft_clip(10.0) 应 < 1.0 |
| 2 | **缺少 MASTER_GAIN** | `synth.rs` | 添加 `MASTER_GAIN = 0.15`，混音后先乘增益再 soft_clip | 3 个 voice 叠加不削波 |
| 3 | **外部 normalize 毁灭动态** | `examples/main.rs` | 删除 normalize，改用固定增益或不做处理 | 输出 WAV 不做过期压缩 |

### 🟡 短期优化（本周）

| # | 问题 | 文件 | 修改内容 |
|---|------|------|---------|
| 4 | **Velocity 衰减不足** | `voice.rs` | `vel_amp` 从 `powi(2)` 改为 `sqrt()`（concave 曲线） |
| 5 | **Voice Stealing 无 fade** | `synth.rs` | 移除最安静 voice，或添加 kill release |
| 6 | **SF2 Release 无条件下限** | `sf2.rs` | `release_secs.max(0.2)` 无条件下限 |

### 🟢 长期改进（后续）

| # | 问题 | 预估工作量 |
|---|------|-----------|
| 7 | 实现真正的 VolumeLimiter（xsynth 风格） | 2h |
| 8 | Channel Volume 二次曲线（powi(2)） | 0.5h |
| 9 | Layer Limit（每 key 最大 voice 数） | 1h |
| 10 | 完整的 SF2 Note-On Modulators | 4h |

---

## 🧠 复盘：为什么 AI 越改越糟？

| 轮次 | AI 修改 | 结果 | 失败原因 |
|------|---------|------|---------|
| 原始 | 直接 mix，无限制 | 电平过高，削波 | 无 gain staging |
| 第 1 轮 | 外部 normalize | 动态被压扁 | normalize 不是解决方案 |
| 第 2 轮 | `.clamp(-1,1)` | 硬削波失真 | 硬截断产生谐波 |
| 第 3 轮 | `soft_clip` (错误公式) | **反向放大，比原来更糟** | AI 从网上抄了一个 "fast tanh" 近似，但没验证其在 \|x\|>3 时的行为 |
| 第 4 轮 | SmoothedValue, RELEASE_TARGET | 爆音无改善 | 没触及真正的根因 |

**根本原因**：
1. **AI 缺乏数学验证能力**：抄了 "fast tanh" 代码，但没有验证 `soft_clip(10.0)` 的输出
2. **症状驱动的修复**：每次只针对用户描述的表面症状（"电平高"→加限制器），没有系统性地设计 gain staging
3. **忽略参考项目的架构**：xsynth 通过 4 层 gain staging（modulator → volume^2 → layer limit → limiter）来避免削波，而不是靠一个"神奇的限幅器"

---

## ✅ 最小可工作的修复（复制粘贴即可）

### 修改 1：`dysonphere-core/src/synth.rs`

```rust
/// 主控增益衰减：为混音预留动态余量。
/// 假设最多约 6 个 voice 同时满幅，0.15 提供约 -16dB 余量。
const MASTER_GAIN: f32 = 0.15;

/// 软限幅：保证 |output| <= 1.0，在 |x|<=1 时近似透明。
#[inline]
fn soft_clip(x: f32) -> f32 {
    let ax = x.abs();
    if ax <= 1.0 {
        x
    } else {
        // 1/x 压缩，单调趋向于 ±1.0
        x.signum() * (1.0 - 0.5 / ax)
    }
}
```

然后在 Mono 和 Stereo 路径中：
```rust
// Mono
*sample = soft_clip(mix * MASTER_GAIN);

// Stereo
chunk[0] = soft_clip(left * MASTER_GAIN);
chunk[1] = soft_clip(right * MASTER_GAIN);
```

### 修改 2：`examples/src/main.rs`

删除 normalize 代码块：
```rust
// 删除以下代码：
// let max = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
// if max > 0.0 {
//     let gain = 0.9 / max;
//     for s in &mut buffer { *s *= gain; }
// }
```

改为直接写入（如果 MASTER_GAIN=0.15，通常不需要额外衰减）：
```rust
for &sample in &buffer {
    let clamped = sample.clamp(-1.0, 1.0);
    writer.write_sample((clamped * i16::MAX as f32) as i16).unwrap();
}
```

### 修改 3：`dysonphere-core/src/voice.rs`

```rust
// 将 vel_amp 从平方改为平方根（concave 曲线，更符合人耳感知）
let vel_amp = (velocity as f32 / 127.0).sqrt();
```

---

*本报告基于对 dysonphere 和 xsynth 的全面静态对比分析，以及对 soft_clip 数学公式的逐点验证生成。上述"最小可工作的修复"经逻辑验证可消除爆音，但建议在实际音频上测试确认。*
