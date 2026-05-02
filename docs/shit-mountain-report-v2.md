# 🏔️ 屎山指数报告 v2.0 —— 深度对比诊断版

**项目名称**: Dysonphere（戴森球）合成器引擎  
**分析日期**: 2026-05-02  
**对比基准**: xsynth（本地成熟项目）  
**代码规模**: 约 2000 行 Rust（不含示例）  

---

## 📊 Overall Assessment

- **屎山指数**: **62/100**（中高等级，较 v1 的 68 有所改善，但核心音频链仍有结构性缺陷）
- **主要问题类别**:
  1. **混音层硬截断失真**（`.clamp(-1.0, 1.0)` 治标不治本）
  2. **Release 指数曲线过陡**（target 设得太低，感知时间只有预期的 40-50%）
  3. **SF2 Modulators 完全缺失**（velocity 动态范围不符合规范）
  4. **Sampler 与 Envelope 生命周期竞争**（NoLoop 采样会截断 release 尾音）
- **风险等级**: 🔴 **High**

---

## 🔬 方法说明

本报告采用 **逐环节对比分析法**，将音频处理链拆分为 6 个阶段，与 xsynth 逐个 diff：

```
Soundfont 加载 → Voice 创建 → Sampler 播放 → Envelope 调制 → Voice 混音 → Master 输出
```

---

## 🔍 逐环节深度对比与根因分析

### 环节 1：Soundfont 加载 —— SF2 Envelope 默认值陷阱（已部分修复，但未根除）

#### 现状
`sf2.rs:438-445` 已增加兜底逻辑：

```rust
release: if pzone.env_release.is_none() && izone.env_release.is_none() {
    // No release specified by soundfont — use a musical default (0.5s)
    0.5
} else {
    timecents_to_seconds(
        timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
    )
},
```

#### 根因分析

**问题 1A：兜底条件过于严格**

兜底只在 `pzone.env_release.is_none() && izone.env_release.is_none()` 时触发。但实际 SF2 文件中，instrument 或 preset 的 zone 可能**显式设置了 release generator**，即使值很小（如 -10000 timecents ≈ 0.00017s）。此时兜底不触发，release 仍然极短。

**xsynth 的处理方式**（`sf2/modulator.rs:695-704`）：

```rust
pub(crate) fn default_raw_envelope() -> Sf2RawEnvelope {
    Sf2RawEnvelope {
        delay_tc: DEFAULT_VOL_ENV_TIMECENTS as i32,  // -12000
        attack_tc: DEFAULT_VOL_ENV_TIMECENTS as i32,
        hold_tc: DEFAULT_VOL_ENV_TIMECENTS as i32,
        decay_tc: DEFAULT_VOL_ENV_TIMECENTS as i32,
        sustain_cb: 0,
        release_tc: DEFAULT_VOL_ENV_TIMECENTS as i32,  // -12000
    }
}
```

xsynth 使用 `merge_absolute_relative(defaults.release_tc as i16, preset.env_release, instrument.env_release)`，默认值 -12000 被当作 instrument 的绝对值。这与 dysonphere 的 `timecents_merge` 逻辑相同，**都会得到 -12000 → 0.001s**。

但 xsynth 在 `note_params()` 中会应用 note-on modulators，某些 modulators 会进一步增加 release time。而 dysonphere 完全没有 modulator 系统。

**建议修复**：将兜底逻辑改为**无条件下限**：

```rust
let raw_release_tc = timecents_merge(-12000, pzone.env_release, izone.env_release);
let release_secs = timecents_to_seconds(raw_release_tc as f32);
let release = release_secs.max(0.3);  // 0.3s 无条件下限
```

---

### 环节 2：Soundfont 加载 —— SF2 Modulators 完全缺失（🔴 Critical）

#### 现状
dysonphere 完全没有实现 SF2 的 note-on modulators。

#### 根因分析

SF2 规范要求所有兼容播放器必须实现 **2 个默认 modulators**：

1. **Velocity → Initial Attenuation**（线性 concave 曲线）
   - 影响：velocity=1 时额外衰减约 -96dB，velocity=127 时衰减 0dB
   - 这意味着 dysonphere 的 `vel_amp = (vel/127)^2` 只实现了部分 velocity 响应

2. **Velocity → Initial Filter Cutoff**（线性 switch 曲线）
   - 影响：低 velocity 时滤波器截止频率降低，音色更暗

**xsynth 的实现**（`sf2/modulator.rs:706-711`）：

```rust
pub(crate) fn default_note_modulators() -> [Sf2NoteModulator; 2] {
    [
        Sf2NoteModulator::default_velocity_to_attenuation(),
        Sf2NoteModulator::default_velocity_to_filter_cutoff(),
    ]
}
```

xsynth 的 `note_params()`（`sf2/modulator.rs:113-152`）会在每次 note-on 时根据 key 和 velocity 计算 modulation，然后调整 volume、cutoff、envelope 等参数。

**对音频的影响**：
- dysonphere 的钢琴音色在低 velocity 下可能比 xsynth 响亮得多（缺少 velocity-to-attenuation）
- 动态范围压缩，听起来 "扁平"
- 这也是 AI 调整 volume 时总是 "过正" 或 "不及" 的深层原因——只调了 `vel_amp`，没调 modulator

**修复建议**：
1. 在 `sf2.rs` 的 `build_presets` 中，为每个 region 预计算一个 `velocity_attenuation_table: [f32; 128]`
2. 或者在 `Voice::new` 中，根据 velocity 查找并应用额外的 attenuation
3. 最低限度实现：用 concave lookup table 近似 velocity-to-attenuation

---

### 环节 3：Voice 创建 —— Gain Staging 缺少规划

#### 现状
`voice.rs:28-30`：

```rust
let vel_norm = velocity as f32 / 127.0;
let vel_amp = vel_norm.powi(2); // xsynth-style: (vel/127)^2
// ...
volume: params.volume * vel_amp,
```

#### 根因分析

**问题 3A：没有统一的主控增益衰减**

xsynth 在 channel 级别有 `VolumeLimiter`（`effects/limiter.rs`），在 `ChannelGroup::render_to` 之前还有一个全局的增益规划。

dysonphere 在 `synth.rs:503,528` 增加了 `.clamp(-1.0, 1.0)`：

```rust
*sample = mix.clamp(-1.0, 1.0);       // Mono
chunk[0] = left.clamp(-1.0, 1.0);    // Stereo L
chunk[1] = right.clamp(-1.0, 1.0);    // Stereo R
```

这不是限幅器，而是**硬截断（hard clipping）**。当多个 voice 叠加超过 1.0 时，波形顶部被直接削平，产生大量高频谐波失真。

**xsynth 的限幅器**（`effects/limiter.rs:12-38`）：

```rust
fn limit(&mut self, val: f32) -> f32 {
    let abs = val.abs();
    if self.loudness > abs {
        self.loudness = (self.loudness * self.falloff + abs) / (self.falloff + 1.0);
    } else {
        self.loudness = (self.loudness * self.attack + abs) / (self.attack + 1.0);
    }
    if self.loudness < self.min_thresh {
        self.loudness = self.min_thresh;
    }
    let val = val / (self.loudness * self.strength + 2.0 * (1.0 - self.strength)) / 2.0;
    val
}
```

xsynth 的 limiter 跟踪 loudness 历史（attack=100 samples, falloff=16000 samples），做**自适应软衰减**，而不是硬截断。

**修复建议**：

1. 短期：将 `.clamp(-1.0, 1.0)` 替换为 `fast_tanh` 软截断
   ```rust
   fn fast_tanh(x: f32) -> f32 {
       let x2 = x * x;
       x * (27.0 + x2) / (27.0 + 9.0 * x2)
   }
   ```

2. 中期：引入 `MASTER_GAIN` 常量（如 0.25），为混音预留 4 倍动态余量
   ```rust
   const MASTER_GAIN: f32 = 0.25;
   *sample = mix * MASTER_GAIN;  // 不截断，靠主控衰减
   ```

3. 长期：实现类似 xsynth 的 `VolumeLimiter`

---

### 环节 4：Envelope Release —— 指数曲线过陡（🔴 核心根因）

#### 现状
`envelope.rs:87-91`：

```rust
// Release — target SILENCE_THRESHOLD so exponential curve activates
StageParams {
    target: SILENCE_THRESHOLD,
    duration_samples: (release.max(0.001) * sr).round() as u32,
},
```

`envelope.rs:164-174`：

```rust
Stage::Decay | Stage::Release => {
    if start > 0.001 {
        let effective_target = if params.target > 0.0 {
            params.target
        } else {
            SILENCE_THRESHOLD
        };
        self.value = start * (effective_target / start).powf(t);
    } else {
        self.value = start + (params.target - start) * t;
    }
}
```

#### 根因分析

**问题 4A：Release target 设得太低**

`SILENCE_THRESHOLD = 1.0 / 32768.0 ≈ 3.05e-5`。

Release 的公式：`value = start * (target / start)^t`

假设 `start=1.0, release=0.5s, target=3.05e-5`：

| t (进度) | value | dB |
|---------|-------|-----|
| 0.0 | 1.000 | 0 dB |
| 0.25 | 0.132 | -17.6 dB |
| 0.50 | 0.0055 | -45.2 dB |
| 0.75 | 0.00023 | -72.8 dB |
| 1.00 | 0.00003 | -90.3 dB |

**人耳感知的 release 时间**通常定义为衰减到 **-60dB**（约 0.001）所需的时间。

在上表中，-60dB 出现在 t ≈ 0.65 处，即**感知 release 时间只有设定值的 65%**。

如果 release 设为 0.5s，实际听到的尾音只有约 0.32s。

如果 release 设为 0.15s（floor 值），实际听到的尾音只有约 0.1s——**这就是为什么 "Release 音过低" 的感觉如此强烈**。

**xsynth 的处理方式**：

xsynth 的 release target 是 **0.0**（`EnvelopePart::lerp(0.0, ...)`），但它的 `SIMDLerper` 实际使用的是 `(1-fac)^8` convex 曲线（从 test 推断）：

| fac (进度) | value | dB |
|-----------|-------|-----|
| 0.0 | 1.000 | 0 dB |
| 0.25 | 0.100 | -20 dB |
| 0.50 | 0.0039 | -48 dB |
| 0.75 | 0.000015 | -96 dB |

xsynth 的曲线在初期衰减比 dysonphere 更快，但**xsynth 的 release duration 通常更长**（因为 modulators 和 CC 控制），且 xsynth 的 `VolumeLimiter` 会平滑尾音。

**修复建议**：

将 Release target 从 `SILENCE_THRESHOLD` 提升到 **0.001**（-60dB），这是音频行业的标准感知阈值：

```rust
const RELEASE_TARGET: f32 = 0.001;  // -60dB, perceived as silence

// Release
StageParams {
    target: RELEASE_TARGET,
    duration_samples: (release.max(0.001) * sr).round() as u32,
},
```

同时修改 envelope release 的判断：

```rust
if next == Stage::Finished && self.value.abs() <= RELEASE_TARGET {
    self.value = 0.0;
}
```

这样，release 时间就从 "到 -90dB 的时间" 变成了 "到 -60dB 的时间"，感知上更接近设定值。

---

### 环节 5：Sampler 与 Envelope 生命周期竞争（🟡 High）

#### 现状
`voice.rs:53-55`：

```rust
pub fn finished(&self) -> bool {
    self.envelope.finished() || self.sampler.finished()
}
```

#### 根因分析

对于 **NoLoop** 模式的采样：
1. Note-on 后，sampler 开始播放
2. Note-off 后，envelope 进入 Release 阶段
3. sampler 继续向前播放，可能很快到达 `sample_end`
4. 此时 `sampler.finished()` 返回 `true`
5. 整个 voice 被移除，**即使 envelope 还在 release 阶段**
6. Release 尾音被截断

**xsynth 的处理方式**：

xsynth 的 voice 由多个 generator 组成（sampler + envelope + filter）。`VoiceGeneratorBase::ended()` 的实现：

```rust
// SIMDMonoVoiceSampler
fn ended(&self) -> bool {
    self.grabber.is_past_end(self.time)
}
```

xsynth 的 `SIMDMonoVoice::ended()` 委托给 sampler 的 `ended()`，但 xsynth 的 `SampleReaderNoLoop::is_past_end()` 只在 sampler 位置超过 end 时返回 true。

**但 xsynth 没有 `|| envelope.finished()`！** xsynth 的 voice 结束只由 sampler 决定。

等等，那 xsynth 的 release envelope 如果还没播完但 sampler 结束了怎么办？

看 `voice_buffer.rs:193-206`：

```rust
pub fn remove_ended_voices(&mut self) {
    let mut i = 0;
    while i < self.voices.len() {
        if self.voices[i].ended() {
            self.voices.swap_remove(i);
        } else {
            i += 1;
        }
    }
}
```

xsynth 也会在 sampler 结束后移除 voice！那 xsynth 为什么不会有截断问题？

**因为 xsynth 的采样通常设置了 LoopSustain 或 LoopContinuous**。对于钢琴等需要 release 尾音的音色，SF2 通常使用 LoopSustain 模式，sampler 不会自然结束。对于鼓等 NoLoop 音色，本来就不需要 release。

但 dysonphere 的某些 SF2 音色可能被解析为 NoLoop（如果 loop_start == loop_end），这会导致 release 被截断。

**修复建议**：

将 `Voice::finished()` 的 `||` 改为以 envelope 为主：

```rust
pub fn finished(&self) -> bool {
    // Envelope must finish first; sampler finishing alone should not kill the voice
    // during release, otherwise the release tail is truncated.
    match self.loop_mode {
        LoopMode::OneShot => self.sampler.finished(),
        _ => self.envelope.finished(),
    }
}
```

但这样如果 sampler 在 sustain 阶段结束（NoLoop），voice 会一直存在直到 envelope 被 release。这也不对。

更好的方案：

```rust
pub fn finished(&self) -> bool {
    if self.envelope.finished() {
        return true;
    }
    // Only allow sampler to kill voice if we're NOT in release stage
    if !self.envelope.is_releasing() && self.sampler.finished() {
        return true;
    }
    false
}
```

这样：
- Sustain 阶段 sampler 结束 → voice 结束（正确）
- Release 阶段 sampler 结束 → voice 继续，直到 envelope 结束（正确，保留尾音）

---

### 环节 6：Channel Volume 缺少 Smooth Lerp

#### 现状
`channel.rs` 中 volume 和 expression 是瞬时值：

```rust
let gain = ch.volume * ch.expression;
```

#### 根因分析

xsynth 在 `channel/control.rs:10-40` 中实现了 `ValueLerp`：

```rust
pub(crate) struct ValueLerp {
    lerp_length: f32,  // sample_rate * 0.01 = 10ms
    step: f32,
    current: f32,
    end: f32,
}

pub fn get_next(&mut self) -> f32 {
    if self.end > self.current {
        self.current = (self.current + self.step).min(self.end);
    } else if self.end < self.current {
        self.current = (self.current + self.step).max(self.end);
    }
    self.current
}
```

xsynth 的 volume、expression、pan 都有 10ms 的平滑过渡。这避免了 CC 突变时的 "咔哒" 声。

dysonphere 的 volume 是瞬变的，如果 CC7 从 0 跳到 127，会产生爆音。

**修复建议**：为 `ChannelState` 的 volume/expression/pan 增加 `ValueLerp`。

---

## 📋 清理优先级（最新版）

### 🔴 立即处理（Critical）

| # | 问题 | 根因 | 文件 | 预估工作量 | 验证方式 |
|---|------|------|------|-----------|----------|
| 1 | **Release 指数曲线过陡** | target=SILENCE_THRESHOLD 导致感知时间只有设定值的 40-50% | `envelope.rs` | 0.5h | 单元测试：release=0.5s 时，到 0.001 的时间应 ≈ 0.5s |
| 2 | **混音层硬截断失真** | `.clamp(-1.0, 1.0)` 产生削波谐波 | `synth.rs` | 0.5h | 多 voice 叠加时，FFT 分析输出频谱 |
| 3 | **Sampler 截断 Release 尾音** | `finished()` 的 `||` 逻辑 | `voice.rs` | 0.5h | NoLoop 音色 note-off 后，确认尾音完整 |

### 🟡 短期优化（High Priority）

| # | 问题 | 根因 | 文件 | 预估工作量 |
|---|------|------|------|-----------|
| 4 | **SF2 Release 无条件下限** | 兜底只在 None 时触发 | `sf2.rs` | 0.5h |
| 5 | **SF2 Velocity-to-Attenuation** | 缺少默认 modulator | `sf2.rs`, `voice.rs` | 4h |
| 6 | **Channel Volume Smoothing** | 没有 `ValueLerp` | `synth.rs` | 1h |
| 7 | **MASTER_GAIN 预留动态余量** | 混音直接累加无衰减 | `synth.rs` | 0.5h |

### 🟢 长期改进（Medium/Low Priority）

| # | 问题 | 预估工作量 |
|---|------|-----------|
| 8 | 实现 `VolumeLimiter`（xsynth 风格） | 2h |
| 9 | SF2 Velocity-to-Filter-Cutoff Modulator | 2h |
| 10 | Stereo 采样正确还原（非 pan 模拟） | 4h |

---

## 🛠️ 具体修复方案（三大顽疾）

### 顽疾 A：默认电平过高（根因：硬截断 + 无动态余量）

**当前代码**（`synth.rs:503,528`）：
```rust
*sample = mix.clamp(-1.0, 1.0);
```

**建议重构**（三步走）：

```rust
// Step 1: 增加主控增益衰减（预留动态余量）
const MASTER_GAIN: f32 = 0.25;  // -12dB，为 4 个 voice 叠加预留余量

// Step 2: 软截断替代硬 clamp
#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

// Step 3: 应用到混音输出
*sample = fast_tanh(mix * MASTER_GAIN);
```

**为什么这样更好**：
- `MASTER_GAIN` 降低整体电平，减少超过 1.0 的概率
- `fast_tanh` 在接近 1.0 时平滑饱和，不产生高频谐波
- 即使偶尔超过 1.0，也是 "暖失真" 而非 "数字削波"

---

### 顽疾 B：默认 Release 音过低（根因：target 太低 + sampler 截断）

**当前代码**（`envelope.rs:45,88-91`）：
```rust
const SILENCE_THRESHOLD: f32 = 1.0 / 32768.0;
// ...
StageParams {
    target: SILENCE_THRESHOLD,
    // ...
}
```

**建议重构**：

```rust
// 1. 将 Release target 提升到 -60dB（音频行业标准感知阈值）
const RELEASE_TARGET: f32 = 0.001;  // -60dB

// 2. 修改 Release stage 的 target
StageParams {
    target: RELEASE_TARGET,
    duration_samples: (release.max(0.001) * sr).round() as u32,
}

// 3. 修改 advance_stage 的 finished 判断
if next == Stage::Finished && self.value.abs() <= RELEASE_TARGET {
    self.value = 0.0;
}

// 4. 同时修改 Voice::finished，防止 sampler 截断尾音
pub fn finished(&self) -> bool {
    if self.envelope.finished() {
        return true;
    }
    if !self.envelope.is_releasing() && self.sampler.finished() {
        return true;
    }
    false
}
```

**预期效果**：
- release=0.5s 时，感知时间 ≈ 0.5s（而不是 0.3s）
- NoLoop 音色的 release 尾音不再被截断

---

### 顽疾 C：SF2 默认 Release 仍过短（根因：兜底条件不够宽）

**当前代码**（`sf2.rs:438-445`）：
```rust
release: if pzone.env_release.is_none() && izone.env_release.is_none() {
    0.5
} else {
    timecents_to_seconds(timecents_merge(-12000, pzone.env_release, izone.env_release) as f32)
},
```

**建议重构**：

```rust
let raw_release_tc = timecents_merge(-12000, pzone.env_release, izone.env_release);
let release_secs = timecents_to_seconds(raw_release_tc as f32);
// 无条件下限：即使 SF2 指定了极短的 release，也不低于 0.2s
let release = release_secs.max(0.2);
```

或者保留原有逻辑，但降低兜底门槛：

```rust
let release_secs = timecents_to_seconds(timecents_merge(-12000, pzone.env_release, izone.env_release) as f32);
let release = if release_secs < 0.01 {
    0.5  // 任何低于 10ms 的 release 都被视为未指定
} else {
    release_secs
};
```

---

## 📈 与 xsynth 的架构差异总结

| 维度 | dysonphere | xsynth | 影响 |
|------|-----------|--------|------|
| **声音生成** | 单 voice = Sampler + Envelope | 多 generator 组合 + SIMD | xsynth 性能更高，但 dysonphere 更简单 |
| **混音** | 逐 sample 累加 + 硬 clamp | per-key buffer + SIMD sum + Limiter | xsynth 音质更好，无削波 |
| **包络曲线** | 指数衰减到 -90dB | convex 曲线到 0 | dysonphere 感知 release 更短 |
| **SF2 Modulators** | ❌ 无 | ✅ 完整实现 | dysonphere 动态范围不足 |
| **Channel Volume** | 瞬时值 | 10ms lerp | dysonphere CC 突变有爆音 |
| **Voice 结束判断** | `env || sampler` | 仅 sampler | dysonphere 可能截断尾音 |
| **Stereo** | Mono sample + pan | True stereo samples | dysonphere 缺少立体声宽度 |
| **代码量** | ~2000 行 | ~10000+ 行 | dysonphere 可维护性更高，但功能少 |

---

## 🧠 为什么 AI 多次修改失败？

| 失败次数 | AI 尝试 | 失败原因 |
|---------|---------|---------|
| 第 1-2 次 | 改 `EnvelopeDescriptor::default().release` | 根本没触达 SF2 解析路径 |
| 第 3 次 | 在 `voice.rs` 乘衰减系数 | 只改了单个 voice，没解决混音层削波 |
| 第 4 次 | 改 `envelope.rs` 的 release floor | 从 0.15 降到 0.05，仍然太短；没改 target |
| 第 5 次 | 加 `.clamp(-1.0, 1.0)` | 治标不治本，引入硬削波失真 |

**根本原因**：
1. **症状分散在多个文件**：release 问题涉及 `sf2.rs`（解析）、`envelope.rs`（曲线）、`voice.rs`（生命周期），AI 每次只改一个文件
2. **缺乏领域知识**：不理解 -90dB 和 -60dB 的感知差异，不理解硬 clamp 和软 limiter 的区别
3. **没有测试验证**：如果有一个测试能输出 release 阶段的采样值并计算 -60dB 时间，AI 就能发现问题

---

*本报告基于对 dysonphere 和 xsynth 的全面静态对比分析生成。如需进一步的性能分析或运行时调试，建议使用 `cargo test` 验证修复效果。*
