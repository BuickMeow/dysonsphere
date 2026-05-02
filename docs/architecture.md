# DysonSphere 音频引擎架构设计

> 目标：Rust 2024 Edition，无 `mod.rs` 风格，高音符数实时播放，现代架构，丰富发声参数。

---

## 1. Rust 2024 Edition + 无 `mod.rs` 风格

Rust 2024 Edition 支持**文件即模块**风格：

```
# 旧风格 (mod.rs)
src/
├── voice/
│   ├── mod.rs          ← 需要这个
│   ├── envelope.rs
│   └── adsr.rs

# 新风格 (Rust 2024, 无 mod.rs)
src/
├── voice.rs            ← 模块入口就是文件本身
├── voice/
│   ├── envelope.rs
│   └── adsr.rs
```

`voice.rs` 里的内容等价于原来 `voice/mod.rs` 的内容，但不需要叫 `mod.rs`。

这会让目录更干净——**一个目录只放子模块，不放入口**。

Cargo.toml 需要声明：
```toml
[package]
edition = "2024"
```

---

## 3. 高音符数（黑乐谱）对架构的影响

黑乐谱的特点是：**极短时间内触发大量 note-on**（可能每秒数万个 voice）。

### xsynth 的应对方式
- 两级 rayon 并行（channel 级 + key 级）
- SIMD monomorphization（每个 voice 内部并行处理 4/8/16 个样本）
- 早期 voice termination（envelope 低于阈值立即结束）

### 它的问题
- **rayon 在 audio callback 里调度** = 非确定性延迟，不适合 DAW 实时
- **Box\<dyn Voice\>** = 每个 voice 堆分配 + 虚调用
- **per-voice 滤波器 clone** = 高内存占用

### DysonSphere 的应对方向

#### 3.1 SoA (Structure of Arrays) 批量处理

不要这样存 voice 数据（AoS）：
```rust
struct Voice {
    pitch: f32,
    phase: f32,
    envelope_amp: f32,
    filter_state: [f32; 2],
}
// Vec<Voice> — 内存不连续，cache miss 严重
```

要这样存（SoA）：
```rust
struct VoiceBank {
    pitches: Vec<f32>,
    phases: Vec<f32>,
    envelope_amps: Vec<f32>,
    filter_states: Vec<[f32; 2]>,
}
// 一次 SIMD 指令处理 N 个 voice 的 pitch
```

**好处**：
- SIMD 一次处理 4/8/16 个 voice 的同一种参数，而不是一个 voice 的 4/8/16 个样本
- 更符合 GPU/DSP 风格，cache 友好
- 黑乐谱场景下，voice 数 >> sample 并行度，SoA 效率更高

**代价**：
- 代码组织方式完全不同，不能照搬 xsynth 的 trait chain
- 需要预分配固定 capacity，不能随意 Box::new()

#### 3.2 固定大小的 Voice Pool

```rust
pub struct VoicePool {
    voices: Vec<VoiceSlot>,  // capacity = max_polyphony（如 65536）
    free_list: Vec<u16>,     // 可用 voice 的索引
    active_mask: BitVec,     // 哪些 voice 正在发声
}
```

- note-on 从 `free_list` pop 一个索引 = O(1)
- note-off 把索引 push 回 `free_list` = O(1)
- 无堆分配，audio callback 安全

#### 3.3 并行策略：只在 channel 级并行，key 级串行

xsynth 在 audio thread 里用 rayon `par_iter_mut()` 并行 key，这有两个问题：
1. 小 buffer 下（如 128 samples），128 个 key 的并行开销 > 收益
2. DAW 的 audio callback 通常已经在独立线程上，再内部并行会争抢 CPU

**建议**：
- channel 级：用 rayon 或自定义 thread pool（在 render thread 里，不在 audio callback 里）
- channel 内部：单线程批量处理所有 active voice（SIMD 向量化）
- 如果 DAW 已经给了一个专用 core，不要在这上面再开线程

**`BufferedRenderer` 模式**（从 xsynth 继承）：
- audio callback 只从 lock-free ring buffer 读预渲染好的样本
- 合成在一个独立线程做，可以用 rayon、可以分配内存
- 这是 DAW 插件的标准做法（VST3/AU/CLAP 都推荐）

---

## 4. 发声结构参数设计

### 4.1 Envelope（包络）

SF2 是 DAHDSR（Delay-Attack-Hold-Decay-Sustain-Release）。
支持**秒数和 BPM 双选项**，DAW 场景下可以用 BPM 同步，通用场景用秒数：

```rust
pub enum TimeUnit {
    Seconds(f32),
    Beats(f32),  // 如 0.5 = 半拍，1.0 = 一拍
}

pub struct EnvelopeParams {
    pub stages: Vec<EnvelopeStage>,  // 不限于 ADSR，可以 3 段、7 段
    pub loop_start: Option<usize>,   // 包络可以循环（如 pad 音色）
    pub release_stage: usize,        // 哪一段开始 release
}

pub struct EnvelopeStage {
    pub target_level: f32,           // 目标电平（相对 0~1）
    pub duration: TimeUnit,          // 支持 Seconds 或 Beats
    pub curve: EnvelopeCurve,        // 曲线类型
}

pub enum EnvelopeCurve {
    Linear,
    Exponential(f32),   // 指数系数
    Bezier(f32, f32),   // 控制点
}

impl EnvelopeStage {
    /// 根据当前 BPM 和采样率计算实际样本数
    pub fn to_samples(&self, bpm: f32, sample_rate: f32) -> usize {
        let seconds = match self.duration {
            TimeUnit::Seconds(s) => s,
            TimeUnit::Beats(beats) => beats * 60.0 / bpm,
        };
        (seconds * sample_rate) as usize
    }
}
```

**BPM 来源**：
- DAW 插件模式下，从宿主获取当前 BPM（VST3/AU/CLAP 都提供）
- 独立播放模式下，默认 120 BPM，可手动设置
- 支持 BPM 变化（DAW 中 tempo track）— 需要实时重新计算剩余样本数

**初期**：先实现 DAHDSR，但内部用 `Vec<EnvelopeStage>` 存，为以后多段包络留扩展。

### 4.2 Filter（滤波器）

| 类型 | 适用场景 | 初期实现？ |
|------|---------|-----------|
| Biquad | 通用，快 | 是 |
| SVF (State Variable) | 调制平滑，无爆音 | 是 |
| ZDF (Zero Delay Feedback) | 模拟感，自振荡 | 否（CPU 高） |

```rust
pub struct FilterParams {
    pub filter_type: FilterType,     // LowPass / HighPass / BandPass / Notch
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub key_tracking: f32,           // 键盘跟踪（ cutoff += key * tracking ）
    pub velocity_tracking: f32,      // 力度跟踪
    pub envelope_amount: f32,        // 包络调制 cutoff
}
```

### 4.3 Unison（齐奏）

```rust
pub struct UnisonParams {
    pub voices: u8,                  // 2~16
    pub detune_cents: f32,           // 总失谐范围（如 ±12 cents）
    pub detune_curve: DetuneCurve,   // Linear / Exponential spread
    pub stereo_spread: f32,          // 声像展宽 0~1
    pub blend: f32,                  // 原音 vs 齐奏混合
    pub phase_mode: PhaseMode,       // Free / Random / Sync
}

pub enum DetuneCurve {
    Linear,        // 均匀分布
    Exponential,   // 中间密，两边疏
    Harmonic,      // 按谐波列分布（适合模拟弦乐堆叠）
}
```

### 4.4 Piano Pedal（钢琴踏板）

```rust
pub struct PedalState {
    pub sustain: bool,      // CC64
    pub sostenuto: bool,    // CC66
    pub soft: bool,         // CC67
}

// Sustain：note-off 后继续保持，直到 pedal release
// Sostenuto：pedal 踩下时正在响的 note 持续，新 note 不受影响
// Soft：降低 velocity 或改变 filter
```

### 4.5 其他可调参数

```rust
pub struct VoiceTimbreParams {
    pub pitch_bend_range: f32,       // semitones，默认 2.0
    pub pitch_envelope_amount: f32,  // 包络对 pitch 的影响
    pub pan_key_tracking: f32,       // 高音偏右、低音偏左（如钢琴）
    pub pan_velocity_tracking: f32,  // 大力偏右
    pub amp_key_tracking: f32,       // 高音衰减（如钢琴）
    pub start_offset_rand: f32,      // 随机起始偏移（防止采样呆板）
    pub legato_mode: LegatoMode,     // 连奏处理方式
}
```

---

## 5. 文件夹结构（Rust 2024 无 mod.rs 风格）

### crates/ vs 扁平结构的取舍

xsynth 是**扁平结构**：`core/`、`soundfonts/` 直接在根目录。两种风格对比：

| 风格 | 优点 | 缺点 |
|------|------|------|
| **扁平**（xsynth 式） | 路径短，crate 之间 import 写起来快 | crate 多了之后根目录乱 |
| **crates/** | 未来加 CLI、FFI、bindings 时结构清晰 | 路径多一层，import 写 `ds-core` 时要习惯 |

**建议**：如果确定只有两个 crate（`ds-core` + `ds-soundfont`），扁平更简洁，和 xsynth 一致；如果以后可能加 `ds-cli`、`ds-vst3` 等，用 `crates/`。

以下是**扁平结构**方案（和 xsynth 一致）：

```
dysonsphere/
├── Cargo.toml                      # workspace manifest
├── ds-core/                        # 核心音频引擎
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                  # 公共 API 导出
│       ├── audio_pipe.rs           # AudioPipe trait
│       ├── audio_stream.rs         # AudioStreamParams, ChannelCount
│       ├── buffer_pool.rs          # 预分配 buffer 池
│       ├── synthesizer.rs          # 顶层合成器
│       ├── channel.rs              # 单 MIDI 通道
│       ├── channel/
│       │   ├── event.rs            # ChannelEvent / ControlEvent
│       │   ├── control.rs          # MIDI CC 解析
│       │   ├── key.rs              # 128 键管理
│       │   ├── voice_pool.rs       # Voice 对象池
│       │   └── soundfont.rs        # 当前通道加载的音色
│       ├── voice.rs                # Voice 模块入口
│       ├── voice/
│       │   ├── bank.rs             # SoA VoiceBank
│       │   ├── spawner.rs          # VoiceSpawner trait
│       │   └── steal.rs            # Voice stealing 策略
│       ├── generator/
│       │   ├── sampler.rs          # 采样器主逻辑
│       │   ├── sampler/
│       │   │   ├── reader.rs       # Sample 读取 + loop
│       │   │   ├── interp.rs       # 插值器 trait + 实现
│       │   │   └── resample.rs     # 采样率转换
│       │   └── oscillator.rs       # （预留）基础振荡器
│       ├── envelope/
│       │   ├── adsr.rs             # 经典 ADSR
│       │   ├── dahdsr.rs           # SF2 六段包络
│       │   ├── multi.rs            # 多段可编程包络
│       │   └── curve.rs            # 曲线数学
│       ├── filter/
│       │   ├── biquad.rs
│       │   ├── svf.rs
│       │   └── params.rs           # FilterParams
│       ├── unison/
│       │   ├── params.rs           # UnisonParams
│       │   ├── detune.rs           # 失谐算法
│       │   └── spread.rs           # 声像展宽
│       ├── pedal/
│       │   ├── sustain.rs          # CC64
│       │   ├── sostenuto.rs        # CC66
│       │   └── soft.rs             # CC67
│       ├── control/
│       │   ├── cc.rs               # CC 状态
│       │   ├── lerp.rs             # 平滑过渡
│       │   └── bend.rs             # Pitch bend
│       ├── effects/
│       │   ├── limiter.rs
│       │   └── filter.rs           # 通道级滤波
│       └── util/
│           ├── simd.rs             # SIMD 抽象（wide crate）
│           ├── math.rs             # cents、dB、pan law
│           └── bitset.rs           # 快速位集操作
│
├── ds-soundfont/                   # SF2/SFZ 解析（无音频引擎依赖）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── sf2.rs                  # SF2 解析入口
│       ├── sf2/
│       │   ├── preset.rs
│       │   ├── instrument.rs
│       │   ├── zone.rs
│       │   └── sample.rs
│       ├── sfz.rs                  # SFZ 解析入口
│       └── sfz/
│           ├── parse.rs
│           ├── region.rs
│           └── grammar.rs
│
└── docs/                           # 设计文档
    ├── architecture.md             # 本文件
    ├── envelope_design.md          # 包络详细设计
    ├── filter_design.md            # 滤波器详细设计
    └── benchmark.md                # 性能基准测试方法
```

---

## 6. 技术选型建议

| 领域 | 推荐 | 理由 |
|------|------|------|
| SIMD | `wide` crate | 跨平台、API 干净、compile-time dispatch |
| 音频解析 | `symphonia` | 已经支持多种格式，xsynth 也在用 |
| SF2 解析 | 自己写 或 用 `soundfont` crate | 自己写更灵活，用 crate 更快 |
| SFZ 解析 | 自己写 | SFZ 规范杂，需要自定义扩展 |
| 线程池 | `rayon`（render thread）| 只在 render thread 用，audio callback 不用 |
| Ring buffer | `crossbeam` | lock-free，DAW 标准 |
| 数学 | `nalgebra` 或自己写 | 音频用不到复杂线性代数，自己写更可控 |
| 测试 | `criterion` | 基准测试标准 |

---

## 7. 初期里程碑建议

### Milestone 0：骨架
- [ ] Workspace 搭建，Rust 2024，两个 crate（ds-core, ds-soundfont）
- [ ] `AudioPipe` trait + `Synthesizer` 空壳
- [ ] CI（build + test + clippy）

### Milestone 1：能出声音
- [ ] SF2 加载（基础 preset/instrument/sample）
- [ ] 单声道采样器（nearest 插值）
- [ ] ADSR 包络
- [ ] 单 channel，单 voice
- [ ] 能 render 一个 NoteOn → 听到声音

### Milestone 2：多 voice +  polyphony
- [ ] VoicePool + voice stealing
- [ ] 多 key 同时发声
- [ ] 基础 CC（volume, pan）
- [ ] `BufferedRenderer` 实时播放

### Milestone 3：现代参数
- [ ] DAHDSR 包络
- [ ] SVF 滤波器 + cutoff/resonance
- [ ] Unison
- [ ] 钢琴踏板（sustain/sostenuto/soft）
- [ ] 丰富发声参数（key tracking、velocity tracking 等）

### Milestone 4：优化
- [ ] Linear/Cubic 插值
- [ ] SoA 批量处理优化
- [ ] 基准测试 vs xsynth

---

## 8. 还需要节能酱决定的事情

1. **Workspace 结构**：
   - 扁平（和 xsynth 一致）：`ds-core/`、`ds-soundfont/` 直接在根目录
   - 或 crates/ 目录包裹

2. **插值器质量策略**：
   - 策略 A：Nearest（预览）→ Linear（实时默认）→ Cubic（高品质模式切换）
   - 策略 B：直接 Linear 做默认，离线才上 Sinc

3. **效果器范围**：
   - 初期只做 per-channel limiter + filter？
   - 还是 per-voice filter 也要初期做？（per-voice filter 黑乐谱场景下很耗）

4. **SF2 modulator 支持程度**：
   - 最小集：只支持 key/velocity → volume/cutoff/pitch（最常见）
   - 完整集：支持 SF2 标准所有 modulator 类型

5. **Voice stealing 策略**：
   - 最简单：kill 最老的 voice
   - 更好：kill 最安静（envelope amp 最低）的 voice
   - 最好：可配置策略

6. **Unison 实现方式**：
   - A：一个"voice"内部生成 N 个 detune 副本（节省 voice pool 开销）
   - B：unison 展开成 N 个独立 voice（简单，但 voice pool 压力大）

告诉我决定后，星星酱就可以开始写代码骨架了～ (｡･ω･｡)ﾉ
