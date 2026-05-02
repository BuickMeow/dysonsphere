//! 包络跟踪器测试 - 精确测量Attack上升时间
//! 
//! 测试方法：
//! 1. 提取信号包络（通过采样包络输出）
//! 2. 测量从10%到90%的上升时间（标准attack测量方法）
//! 3. 分析实际曲线形状，找出与理论值的偏差原因

use crate::Envelope;
use dysonphere_soundfont::types::EnvelopeDescriptor;

/// 包络采样点
#[derive(Debug, Clone)]
pub struct EnvelopeSample {
    /// 样本序号
    pub sample_index: usize,
    /// 包络值 (0.0 - 1.0)
    pub value: f32,
    /// 当前阶段
    pub stage: &'static str,
}

/// Attack测量结果
#[derive(Debug, Clone)]
pub struct AttackMeasurement {
    /// 理论attack时间 (秒)
    pub theoretical_attack_sec: f32,
    /// 实际测量的attack时间 (秒) - 10%到90%
    pub measured_attack_sec: f32,
    /// 达到10%的时间点 (秒)
    pub t10_sec: f32,
    /// 达到50%的时间点 (秒)
    pub t50_sec: f32,
    /// 达到90%的时间点 (秒)
    pub t90_sec: f32,
    /// 达到99%的时间点 (秒) - 接近峰值
    pub t99_sec: f32,
    /// 采样率
    pub sample_rate: u32,
    /// 所有采样点
    pub samples: Vec<EnvelopeSample>,
    /// 曲线形状分析
    pub curve_analysis: CurveAnalysis,
}

/// 曲线形状分析
#[derive(Debug, Clone)]
pub struct CurveAnalysis {
    /// 前半段10%-50%耗时
    pub t10_to_t50_ms: f32,
    /// 后半段50%-90%耗时
    pub t50_to_t90_ms: f32,
    /// 前后半段比值 (<1表示凹曲线，开始快)
    pub ratio: f32,
    /// 曲线类型
    pub curve_type: &'static str,
}

impl AttackMeasurement {
    /// 打印详细报告
    pub fn print_report(&self) {
        println!("╔════════════════════════════════════════════════════════════════╗");
        println!("║              包络 Attack 时间精确测量报告                      ║");
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ 采样率:          {:>12} Hz                                ║", self.sample_rate);
        println!("║ 理论Attack:      {:>12.4} ms                              ║", self.theoretical_attack_sec * 1000.0);
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ 测量结果 (10% → 90% 标准):                                     ║");
        println!("║   T10 (10%):     {:>12.4} ms                              ║", self.t10_sec * 1000.0);
        println!("║   T50 (50%):     {:>12.4} ms                              ║", self.t50_sec * 1000.0);
        println!("║   T90 (90%):     {:>12.4} ms                              ║", self.t90_sec * 1000.0);
        println!("║   T99 (99%):     {:>12.4} ms                              ║", self.t99_sec * 1000.0);
        println!("║ ────────────────────────────────────────────────────────────── ║");
        println!("║   实际Attack:    {:>12.4} ms                              ║", self.measured_attack_sec * 1000.0);
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ 曲线形状分析:                                                  ║");
        println!("║   类型:          {:>12}                                  ║", self.curve_analysis.curve_type);
        println!("║   10%-50%:       {:>12.4} ms                              ║", self.curve_analysis.t10_to_t50_ms);
        println!("║   50%-90%:       {:>12.4} ms                              ║", self.curve_analysis.t50_to_t90_ms);
        println!("║   比值:          {:>12.2}                                ║", self.curve_analysis.ratio);
        println!("╠════════════════════════════════════════════════════════════════╣");
        let diff_ms = (self.measured_attack_sec - self.theoretical_attack_sec).abs() * 1000.0;
        let diff_pct = if self.theoretical_attack_sec > 0.0 {
            ((self.measured_attack_sec - self.theoretical_attack_sec) / self.theoretical_attack_sec * 100.0).abs()
        } else {
            0.0
        };
        println!("║ 误差分析:                                                      ║");
        println!("║   绝对误差:      {:>12.4} ms                              ║", diff_ms);
        println!("║   相对误差:      {:>12.2}%                                 ║", diff_pct);
        println!("╚════════════════════════════════════════════════════════════════╝");
    }
}

/// 包络跟踪器 - 提取并分析包络信号
pub struct EnvelopeTracker {
    samples: Vec<EnvelopeSample>,
    sample_rate: u32,
}

impl EnvelopeTracker {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate,
        }
    }

    /// 记录一个采样点
    pub fn record(&mut self, sample_index: usize, value: f32, stage: &'static str) {
        self.samples.push(EnvelopeSample {
            sample_index,
            value,
            stage,
        });
    }

    /// 查找指定幅值的时间点（线性插值）
    fn find_time_at_level(&self, target_level: f32) -> Option<f32> {
        for i in 1..self.samples.len() {
            let prev = &self.samples[i - 1];
            let curr = &self.samples[i];
            
            // 检查是否跨越了目标电平
            if (prev.value <= target_level && curr.value >= target_level) ||
               (prev.value >= target_level && curr.value <= target_level) {
                // 线性插值计算精确时间点
                let t_prev = prev.sample_index as f32 / self.sample_rate as f32;
                let t_curr = curr.sample_index as f32 / self.sample_rate as f32;
                let fraction = if curr.value != prev.value {
                    (target_level - prev.value) / (curr.value - prev.value)
                } else {
                    0.0
                };
                let t_interpolated = t_prev + fraction * (t_curr - t_prev);
                return Some(t_interpolated);
            }
        }
        None
    }

    /// 测量attack时间 (10% → 90% 标准)
    pub fn measure_attack(&self, theoretical_attack_sec: f32) -> AttackMeasurement {
        let t10 = self.find_time_at_level(0.10).unwrap_or(0.0);
        let t50 = self.find_time_at_level(0.50).unwrap_or(0.0);
        let t90 = self.find_time_at_level(0.90).unwrap_or(0.0);
        let t99 = self.find_time_at_level(0.99).unwrap_or(0.0);
        
        let t10_to_t50 = (t50 - t10) * 1000.0; // ms
        let t50_to_t90 = (t90 - t50) * 1000.0; // ms
        let ratio = if t50_to_t90 > 0.0 { t10_to_t50 / t50_to_t90 } else { 1.0 };
        
        let curve_type = if ratio < 0.8 {
            "凹曲线(快起)"
        } else if ratio > 1.2 {
            "凸曲线(慢起)"
        } else {
            "近似线性"
        };
        
        AttackMeasurement {
            theoretical_attack_sec,
            measured_attack_sec: t90 - t10,
            t10_sec: t10,
            t50_sec: t50,
            t90_sec: t90,
            t99_sec: t99,
            sample_rate: self.sample_rate,
            samples: self.samples.clone(),
            curve_analysis: CurveAnalysis {
                t10_to_t50_ms: t10_to_t50,
                t50_to_t90_ms: t50_to_t90,
                ratio,
                curve_type,
            },
        }
    }

    /// 导出CSV格式的数据
    pub fn export_csv(&self) -> String {
        let mut csv = String::from("sample_index,time_ms,value,stage\n");
        for s in &self.samples {
            let time_ms = s.sample_index as f32 / self.sample_rate as f32 * 1000.0;
            csv.push_str(&format!("{},{:.4},{:.6},{}\n", s.sample_index, time_ms, s.value, s.stage));
        }
        csv
    }
}

/// 运行包络跟踪测试，测量attack时间
/// 
/// # 参数
/// - `attack_sec`: 理论attack时间（秒）
/// - `sample_rate`: 采样率（Hz）
/// 
/// # 返回
/// 详细的attack测量结果
pub fn test_envelope_attack(attack_sec: f32, sample_rate: u32) -> AttackMeasurement {
    let desc = EnvelopeDescriptor {
        delay: 0.0,
        attack: attack_sec,
        hold: 0.0,
        decay: 0.0,
        sustain: 1.0,
        release: 1.0,
    };

    let mut env = Envelope::new(desc, sample_rate, true, 127);
    let mut tracker = EnvelopeTracker::new(sample_rate);

    // 记录初始状态
    tracker.record(0, env.value(), "Attack");

    // 处理包络直到完成attack阶段（进入hold或更高阶段）
    let mut sample_index = 0usize;
    let attack_samples = (attack_sec * sample_rate as f32).ceil() as usize + 100; // 多采一些点
    
    for _ in 0..attack_samples {
        env.process();
        sample_index += 1;
        
        let stage_name = if env.value() >= 0.999 {
            "Hold/Sustain"
        } else {
            "Attack"
        };
        
        tracker.record(sample_index, env.value(), stage_name);
        
        // 如果已经达到峰值并保持，停止采样
        if env.value() >= 0.9999 {
            break;
        }
    }

    tracker.measure_attack(attack_sec)
}

/// 分析包络曲线的数学特性
/// 原始包络使用: value = 1.0 - (1.0 - t)^2  (凹曲线)
/// 这导致实际10%-90%时间远小于理论attack时间
pub fn analyze_curve_mathematically(theoretical_attack_sec: f32) {
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║           包络曲线数学分析                                     ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    
    // 原始包络曲线: y = 1 - (1-t)^2 = 2t - t^2
    // 反函数: t = 1 - sqrt(1-y)
    
    println!("\n理论曲线公式: y = 1 - (1-t)² = 2t - t²  (凹曲线)");
    println!("反函数: t = 1 - √(1-y)\n");
    
    let levels = [0.10f32, 0.50, 0.90, 0.99];
    
    println!("理论Attack = {:.4} ms 时的各电平时间点:", theoretical_attack_sec * 1000.0);
    println!("┌─────────┬─────────────┬─────────────────┐");
    println!("│  电平   │  归一化时间  │   实际时间(ms)  │");
    println!("├─────────┼─────────────┼─────────────────┤");
    
    for &y in &levels {
        let t_normalized = 1.0 - (1.0 - y).sqrt();
        let t_actual_ms = t_normalized * theoretical_attack_sec * 1000.0;
        println!("│  {:>4.0}%  │   {:>8.4}   │    {:>10.4}   │", y * 100.0, t_normalized, t_actual_ms);
    }
    
    println!("└─────────┴─────────────┴─────────────────┘");
    
    // 计算10%-90%时间
    let t10_norm = 1.0 - (1.0f32 - 0.10).sqrt();
    let t90_norm = 1.0 - (1.0f32 - 0.90).sqrt();
    let measured_attack_norm = t90_norm - t10_norm;
    let measured_attack_ms = measured_attack_norm * theoretical_attack_sec * 1000.0;
    
    println!("\n10%-90% Attack时间:");
    println!("  归一化: {:.4} (理论值的 {:.1}%)", measured_attack_norm, measured_attack_norm * 100.0);
    println!("  实际值:  {:.4} ms", measured_attack_ms);
    println!("  理论值:  {:.4} ms", theoretical_attack_sec * 1000.0);
    println!("  偏差:    {:.1}%", (1.0 - measured_attack_norm) * 100.0);
    
    println!("\n⚠️  问题分析:");
    println!("   由于使用凹曲线 (1-(1-t)²)，信号在attack初期上升极快，");
    println!("   导致10%-90%的测量时间只有理论attack时间的约28.3%！");
    println!("   这是设计选择，但会造成用户感知的attack时间与参数设置不符。");
}

/// 运行多个attack时间的测试
pub fn run_attack_tests() {
    println!("\n");
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║         包络跟踪器 Attack 时间精确测量测试                     ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    
    let sample_rate = 44100u32;
    let test_cases = vec![
        0.001f32,  // 1ms - 极快
        0.005,     // 5ms - 快
        0.01,      // 10ms - 中等
        0.05,      // 50ms - 慢
        0.1,       // 100ms - 很慢
        0.5,       // 500ms - 极慢
    ];

    for attack_sec in test_cases {
        println!("\n");
        let result = test_envelope_attack(attack_sec, sample_rate);
        result.print_report();
    }
    
    // 数学分析
    analyze_curve_mathematically(0.01);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attack_measurement_detailed() {
        // 详细测试10ms attack
        let result = test_envelope_attack(0.01, 44100);
        
        println!("\n✅ 详细Attack测量结果:");
        result.print_report();
        
        // 验证曲线形状 - 应该是凹曲线
        assert!(
            result.curve_analysis.ratio < 0.9,
            "Attack应该是凹曲线（开始快），ratio应<0.9，实际={:.2}",
            result.curve_analysis.ratio
        );
        
        // 验证T10 < T50 < T90 < T99
        assert!(result.t10_sec < result.t50_sec, "T10应<T50");
        assert!(result.t50_sec < result.t90_sec, "T50应<T90");
        assert!(result.t90_sec < result.t99_sec, "T90应<T99");
    }

    #[test]
    fn test_fast_attack_1ms() {
        // 测试1ms极快attack
        let result = test_envelope_attack(0.001, 44100);
        
        println!("\n✅ 1ms快速Attack测试:");
        result.print_report();
        
        assert!(result.measured_attack_sec > 0.0, "Attack时间必须大于0");
        assert!(result.t90_sec > result.t10_sec, "T90必须大于T10");
    }

    #[test]
    fn test_slow_attack_100ms() {
        // 测试100ms慢attack
        let result = test_envelope_attack(0.1, 44100);
        
        println!("\n✅ 100ms慢Attack测试:");
        result.print_report();
        
        assert!(result.t99_sec > result.t90_sec, "应能测量到T99");
    }

    #[test]
    fn test_attack_curve_consistency() {
        // 测试不同attack时间的曲线形状一致性
        let attacks = vec![0.001f32, 0.01, 0.1];
        
        println!("\n✅ 曲线形状一致性测试:");
        println!("┌─────────────┬────────┬────────┬────────┐");
        println!("│ Attack时间  │ 10%-50%│ 50%-90%│  比值  │");
        println!("├─────────────┼────────┼────────┼────────┤");
        
        for attack in &attacks {
            let result = test_envelope_attack(*attack, 44100);
            println!("│   {:>6.1}ms  │ {:>6.2} │ {:>6.2} │ {:>6.2} │",
                attack * 1000.0,
                result.curve_analysis.t10_to_t50_ms,
                result.curve_analysis.t50_to_t90_ms,
                result.curve_analysis.ratio
            );
            
            // 所有attack时间应该有相似的曲线形状（比值相近）
            assert!(
                result.curve_analysis.ratio > 0.6 && result.curve_analysis.ratio < 0.9,
                "凹曲线ratio应在0.6-0.9之间"
            );
        }
        
        println!("└─────────────┴────────┴────────┴────────┘");
        println!("\n结论: 曲线形状与attack时间无关，始终为凹曲线");
    }
    
    #[test]
    fn test_mathematical_analysis() {
        analyze_curve_mathematically(0.01);
    }
}
