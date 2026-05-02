//! 精确数值匹配验证
//!
//! 逐采样点对比修复前后的数值，确保修复生效

use crate::Envelope;
use dysonphere_soundfont::types::EnvelopeDescriptor;

/// 计算凹曲线的理论值（修复前）
fn concave_curve(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(2)
}

/// 计算线性曲线的理论值（修复后）
fn linear_curve(t: f32) -> f32 {
    t
}

/// 精确数值验证
pub fn precise_numeric_verification() {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║     精确数值匹配验证                                         ║");
    println!("╚════════════════════════════════════════════════════════════════╝");

    let sample_rate = 44100u32;
    let attack_sec = 0.01f32; // 10ms
    let duration = (attack_sec * sample_rate as f32) as usize;

    let desc = EnvelopeDescriptor {
        delay: 0.0,
        attack: attack_sec,
        hold: 0.0,
        decay: 0.0,
        sustain: 1.0,
        release: 1.0,
    };

    println!("\n【逐采样点对比 - 前20个】");
    println!("┌────────┬─────────────┬─────────────┬─────────────┬─────────────┐");
    println!("│ Sample │  凹曲线(旧) │  线性(新)   │  实际包络值 │  匹配?      │");
    println!("├────────┼─────────────┼─────────────┼─────────────┼─────────────┤");

    let mut env = Envelope::new(desc, sample_rate, true, 127);

    for i in 0..=20 {
        let t = i as f32 / duration as f32;
        let concave = concave_curve(t);
        let linear = linear_curve(t);
        let actual = env.value();

        // 判断是否匹配线性（误差<0.0001）
        let matches_linear = (actual - linear).abs() < 0.0001;
        let matches_concave = (actual - concave).abs() < 0.0001;

        let status = if matches_linear {
            "✓ 线性"
        } else if matches_concave {
            "✗ 凹曲线"
        } else {
            "? 其他"
        };

        println!("│ {:>6} │ {:>11.6} │ {:>11.6} │ {:>11.6} │ {:<11} │",
            i, concave, linear, actual, status);

        if i < 20 {
            env.process();
        }
    }

    println!("└────────┴─────────────┴─────────────┴─────────────┴─────────────┘");

    // 统计匹配情况
    println!("\n【统计信息】");
    let mut linear_count = 0;
    let mut concave_count = 0;
    let mut other_count = 0;

    let mut env2 = Envelope::new(desc, sample_rate, true, 127);
    for i in 0..=duration {
        let t = i as f32 / duration as f32;
        let concave = concave_curve(t);
        let linear = linear_curve(t);
        let actual = env2.value();

        let err_linear = (actual - linear).abs();
        let err_concave = (actual - concave).abs();

        if err_linear < 0.001 {
            linear_count += 1;
        } else if err_concave < 0.001 {
            concave_count += 1;
        } else {
            other_count += 1;
        }

        if i < duration {
            env2.process();
        }
    }

    println!("  总采样点数: {}", duration + 1);
    println!("  匹配线性:   {} ({:.1}%)", linear_count, linear_count as f32 / (duration + 1) as f32 * 100.0);
    println!("  匹配凹曲线: {} ({:.1}%)", concave_count, concave_count as f32 / (duration + 1) as f32 * 100.0);
    println!("  其他:       {} ({:.1}%)", other_count, other_count as f32 / (duration + 1) as f32 * 100.0);

    if linear_count > concave_count {
        println!("\n✅ 验证通过！包络已改为线性曲线");
    } else {
        println!("\n❌ 验证失败！包络仍然是凹曲线");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_precise_verification() {
        precise_numeric_verification();
    }

    #[test]
    fn test_sample_by_sample_match() {
        // 逐采样点验证是否匹配线性曲线
        let sample_rate = 44100u32;
        let attack_sec = 0.01f32;
        let duration = (attack_sec * sample_rate as f32) as usize;

        let desc = EnvelopeDescriptor {
            delay: 0.0,
            attack: attack_sec,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 1.0,
        };

        let mut env = Envelope::new(desc, sample_rate, true, 127);

        // 验证前100个采样点
        for i in 0..=100 {
            let t = i as f32 / duration as f32;
            let expected_linear = linear_curve(t);
            let actual = env.value();
            let error = (actual - expected_linear).abs();

            // 允许0.1%的误差
            assert!(
                error < 0.001,
                "Sample {}: expected {:.6}, got {:.6}, error {:.6}",
                i, expected_linear, actual, error
            );

            if i < 100 {
                env.process();
            }
        }

        println!("✅ 前100个采样点全部匹配线性曲线！");
    }

    #[test]
    fn test_not_concave_anymore() {
        // 验证不再是凹曲线
        let sample_rate = 44100u32;
        let attack_sec = 0.01f32;
        let duration = (attack_sec * sample_rate as f32) as usize;

        let desc = EnvelopeDescriptor {
            delay: 0.0,
            attack: attack_sec,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 1.0,
        };

        let mut env = Envelope::new(desc, sample_rate, true, 127);

        // 验证sample 10
        for _ in 0..10 {
            env.process();
        }

        let t = 10.0 / duration as f32;
        let expected_linear = linear_curve(t);
        let expected_concave = concave_curve(t);
        let actual = env.value();

        let err_linear = (actual - expected_linear).abs();
        let err_concave = (actual - expected_concave).abs();

        println!("Sample 10:");
        println!("  实际值:       {:.6}", actual);
        println!("  线性期望值:   {:.6}", expected_linear);
        println!("  凹曲线期望值: {:.6}", expected_concave);
        println!("  与线性误差:   {:.6}", err_linear);
        println!("  与凹曲线误差: {:.6}", err_concave);

        // 应该更接近线性而非凹曲线
        assert!(
            err_linear < err_concave,
            "修复后应该更接近线性！线性误差={:.6}, 凹曲线误差={:.6}",
            err_linear, err_concave
        );

        println!("✅ 验证通过！不再是凹曲线");
    }
}
