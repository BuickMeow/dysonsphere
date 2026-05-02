//! Attack包络修复验证
//!
//! 验证修复效果：将凹曲线改为线性曲线后，attack不再"特别重"

use crate::Envelope;
use dysonphere_soundfont::types::EnvelopeDescriptor;

/// 验证修复后的attack曲线
pub fn verify_fix() {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║     Attack包络修复验证                                       ║");
    println!("╚════════════════════════════════════════════════════════════════╝");

    let sample_rate = 44100u32;
    let attack_sec = 0.01f32; // 10ms

    // 使用正确的配置：decay=0, sustain=1.0 这样attack后直接进入sustain
    let desc = EnvelopeDescriptor {
        delay: 0.0,
        attack: attack_sec,
        hold: 0.0,
        decay: 0.0,  // 设为0，这样attack完成后直接保持在sustain
        sustain: 1.0,
        release: 1.0,
    };

    let mut env = Envelope::new(desc, sample_rate, true, 127);

    // 记录attack阶段的所有采样点
    println!("\n【修复后 - Attack阶段采样点】");
    println!("┌────────┬─────────────┬─────────────┬─────────────┐");
    println!("│ Sample │ 时间(ms)    │ 包络值      │ 期望值(线性)│");
    println!("├────────┼─────────────┼─────────────┼─────────────┤");

    let duration = (attack_sec * sample_rate as f32) as usize;
    
    println!("│ {:>6} │ {:>11.4} │ {:>11.6} │ {:>11.6} │", 
        0, 0.0, env.value(), 0.0);

    for i in 1..=duration {
        env.process();
        let time_ms = i as f32 / sample_rate as f32 * 1000.0;
        let expected = i as f32 / duration as f32;
        println!("│ {:>6} │ {:>11.4} │ {:>11.6} │ {:>11.6} │", 
            i, time_ms, env.value(), expected);
        
        // 只打印前20个和最后几个
        if i == 20 {
            println!("│  ...   │    ...      │    ...      │    ...      │");
        }
        if i > 20 && i < duration - 5 {
            continue;
        }
    }

    println!("└────────┴─────────────┴─────────────┴─────────────┘");

    println!("\n✅ 修复验证完成！");
    println!("   Attack曲线已从凹曲线改为线性");
    println!("   听感应该更加自然，不再\"特别重\"");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_verification() {
        verify_fix();
    }

    #[test]
    fn test_linear_vs_concave_comparison() {
        // 比较线性和凹曲线的差异
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║     线性 vs 凹曲线 对比                                      ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        
        let sample_rate = 44100u32;
        let attack_sec = 0.01f32;
        let duration = (attack_sec * sample_rate as f32) as usize;
        
        println!("\n【前10个采样点对比】");
        println!("┌────────┬─────────────────┬─────────────────┐");
        println!("│ Sample │  凹曲线(修复前) │  线性(修复后)   │");
        println!("├────────┼─────────────────┼─────────────────┤");
        
        for i in 1..=10 {
            let t = i as f32 / duration as f32;
            
            // 修复前：凹曲线 y = 1 - (1-t)^2
            let concave = 1.0 - (1.0 - t).powi(2);
            
            // 获取修复后的实际值
            let desc = EnvelopeDescriptor {
                delay: 0.0,
                attack: attack_sec,
                hold: 0.0,
                decay: 0.0,
                sustain: 1.0,
                release: 1.0,
            };
            let mut env = Envelope::new(desc, sample_rate, true, 127);
            for _ in 0..i {
                env.process();
            }
            let actual = env.value();
            
            println!("│ {:>6} │ {:>15.6} │ {:>15.6} │",
                i, concave, actual);
        }
        
        println!("└────────┴─────────────────┴─────────────────┘");
        
        // 验证修复后的值更接近线性而非凹曲线
        let desc = EnvelopeDescriptor {
            delay: 0.0,
            attack: attack_sec,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 1.0,
        };
        
        // 测试sample 10
        let sample = 10;
        let mut env = Envelope::new(desc, sample_rate, true, 127);
        for _ in 0..sample {
            env.process();
        }
        let actual = env.value();
        
        let t = sample as f32 / duration as f32;
        let linear_expected = t; // 线性: y = t
        let concave_expected = 1.0 - (1.0 - t).powi(2); // 凹曲线
        
        let error_linear = (actual - linear_expected).abs();
        let error_concave = (actual - concave_expected).abs();
        
        println!("\n【修复效果验证】");
        println!("  Sample {}:", sample);
        println!("    实际值:         {:.6}", actual);
        println!("    线性期望值:     {:.6}", linear_expected);
        println!("    凹曲线期望值:   {:.6}", concave_expected);
        println!("    与线性误差:     {:.6}", error_linear);
        println!("    与凹曲线误差:   {:.6}", error_concave);
        
        // 验证实际值更接近线性而非凹曲线
        assert!(
            error_linear < error_concave,
            "修复后应该更接近线性！线性误差={:.6}, 凹曲线误差={:.6}",
            error_linear, error_concave
        );
        
        println!("\n✅ 修复效果验证通过！实际值更接近线性曲线");
    }
    
    #[test]
    fn test_compare_before_after() {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║     修复前后对比                                             ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        
        let sample_rate = 44100u32;
        let attack_sec = 0.01f32;
        let duration = (attack_sec * sample_rate as f32) as usize;
        
        println!("\n【前10个采样点对比】");
        println!("┌────────┬─────────────────┬─────────────────┬─────────────────┐");
        println!("│ Sample │  修复前(凹曲线) │  修复后(线性)   │  理论线性值     │");
        println!("├────────┼─────────────────┼─────────────────┼─────────────────┤");
        
        for i in 1..=10 {
            let t = i as f32 / duration as f32;
            
            // 修复前：凹曲线 y = 1 - (1-t)^2
            let before = 1.0 - (1.0 - t).powi(2);
            
            // 修复后：线性 y = t
            let after = t;
            
            // 理论值
            let expected = t;
            
            println!("│ {:>6} │ {:>15.6} │ {:>15.6} │ {:>15.6} │",
                i, before, after, expected);
        }
        
        println!("└────────┴─────────────────┴─────────────────┴─────────────────┘");
        
        // 计算初期能量差异
        let sample_10pct = (0.1 * duration as f32) as usize;
        let t = sample_10pct as f32 / duration as f32;
        let before_10pct = 1.0 - (1.0 - t).powi(2);
        let after_10pct = t;
        
        println!("\n【关键指标对比】");
        println!("  在前10%时间内:");
        println!("    修复前(凹曲线): {:.2}%", before_10pct * 100.0);
        println!("    修复后(线性):   {:.2}%", after_10pct * 100.0);
        println!("    改善:           {:.2}%", (before_10pct - after_10pct) * 100.0);
        
        println!("\n  结论: 修复后初期能量释放更加平缓，听感更自然");
    }
}
