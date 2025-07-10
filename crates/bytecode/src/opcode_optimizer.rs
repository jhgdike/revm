use thiserror::Error;
use crate::opcode as op;

/// superinstruction in revm

/// 自定义优化 opcode 的最小与最大取值范围。
pub const MIN_OPTIMIZED_OPCODE: u8 = 0xB0;
/// superinstruction max opcode
pub const MAX_OPTIMIZED_OPCODE: u8 = 0xC8;

/// FailPreprocessing Fusion err
#[derive(Debug, Error)]
pub enum FusionError {
    #[error("optimized opcode already present (pre-processing fail)")]
    /// Input bytecode already contains optimized opcodes; fusion aborted.
    FailPreprocessing,
}

/// 对字节码进行模式匹配融合，生成新的字节码副本。
///
/// 1. `code` 本身保持不变，返回新的 `Vec<u8>`；
/// 2. 若发现字节码中已包含任何优化 opcode（0xB0~0xC8），直接返回 `FusionError::FailPreprocessing`；
/// 3. 若在遍历过程中遇到 `INVALID`(0xFE) 则提早终止并返回当前结果。
pub fn do_code_fusion(code: &[u8]) -> Result<Vec<u8>, FusionError> {
    let mut fused = code.to_vec();
    let mut i = 0usize;
    while i < fused.len() {
        let cur = i;

        // 1. 提前终止：若遇到 INVALID。
        if fused[cur] == op::INVALID {
            return Ok(fused);
        }

        // 2. 预处理：若已包含优化 opcode，直接报错。
        if (MIN_OPTIMIZED_OPCODE..=MAX_OPTIMIZED_OPCODE).contains(&fused[cur]) {
            return Err(FusionError::FailPreprocessing);
        }

        // ----------------------------
        // 15-byte 融合
        // PUSH1 _ CALLDATALOAD PUSH1 _ SHR DUP1 PUSH4 _ _ _ _ _ GT PUSH2 _ _
        // ----------------------------
        if cur + 15 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::PUSH1
                && c(2) == op::CALLDATALOAD
                && c(3) == op::PUSH1
                && c(5) == op::SHR
                && c(6) == op::DUP1
                && c(7) == op::PUSH4
                && c(12) == op::GT
                && c(13) == op::PUSH2
            {
                fused[cur] = op::PUSH1CALLDATALOADPUSH1SHRDUP1PUSH4GTPUSH2;
                for off in [2, 3, 5, 6, 7, 12, 13] {
                    fused[cur + off] = op::NOP;
                }
                i += 15;
                continue;
            }
        }

        // ----------------------------
        // 12-byte 融合
        // SWAP1 PUSH1 _ DUP1 NOT SWAP2 ADD AND DUP2 ADD SWAP1 DUP2 LT
        // ----------------------------
        if cur + 12 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::SWAP1
                && c(1) == op::PUSH1
                && c(3) == op::DUP1
                && c(4) == op::NOT
                && c(5) == op::SWAP2
                && c(6) == op::ADD
                && c(7) == op::AND
                && c(8) == op::DUP2
                && c(9) == op::ADD
                && c(10) == op::SWAP1
                && c(11) == op::DUP2
                && c(12) == op::LT
            {
                fused[cur] = op::SWAP1PUSH1DUP1NOTSWAP2ADDANDDUP2ADDSWAP1DUP2LT;
                for off in [1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
                    fused[cur + off] = op::NOP;
                }
                i += 12;
                continue;
            }
        }

        // ----------------------------
        // 9-byte 融合
        // DUP1 PUSH4 _ _ _ _ EQ PUSH2 _ _
        // ----------------------------
        if cur + 9 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::DUP1 && c(1) == op::PUSH4 && c(6) == op::EQ && c(7) == op::PUSH2 {
                fused[cur] = op::DUP1PUSH4EQPUSH2;
                for off in [1, 6, 7] {
                    fused[cur + off] = op::NOP;
                }
                i += 9;
                continue;
            }
        }

        // ----------------------------
        // 7-byte 融合
        // PUSH1 _ PUSH1 _ PUSH1 _ SHL SUB
        // ----------------------------
        if cur + 7 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::PUSH1
                && c(2) == op::PUSH1
                && c(4) == op::PUSH1
                && c(6) == op::SHL
                && c(7) == op::SUB
            {
                fused[cur] = op::PUSH1PUSH1PUSH1SHLSUB;
                for off in [2, 4, 6, 7] {
                    fused[cur + off] = op::NOP;
                }
                i += 7;
                continue;
            }
        }

        // ----------------------------
        // 6-byte 融合 (AND DUP2 ADD SWAP1 DUP2 LT)
        // ----------------------------
        if cur + 5 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::AND
                && c(1) == op::DUP2
                && c(2) == op::ADD
                && c(3) == op::SWAP1
                && c(4) == op::DUP2
                && c(5) == op::LT
            {
                fused[cur] = op::ANDDUP2ADDSWAP1DUP2LT;
                for off in [1, 2, 3, 4, 5] {
                    fused[cur + off] = op::NOP;
                }
                i += 5;
                continue;
            }
        }

        // ----------------------------
        // 5-byte 融合
        // （1）AND SWAP1 POP SWAP2 SWAP1
        // （2）ISZERO PUSH2 _ _ JUMPI  ➜ JUMPIFZERO
        // （3）DUP2 MSTORE PUSH1 _ ADD ➜ DUP2MSTOREPUSH1ADD
        // ----------------------------
        if cur + 4 < fused.len() {
            let c = |o: usize| fused[cur + o];
            // (1)
            if c(0) == op::AND
                && c(1) == op::SWAP1
                && c(2) == op::POP
                && c(3) == op::SWAP2
                && c(4) == op::SWAP1
            {
                fused[cur] = op::ANDSWAP1POPSWAP2SWAP1;
                for off in [1, 2, 3, 4] {
                    fused[cur + off] = op::NOP;
                }
                i += 4;
                continue;
            }
            // (2)
            if c(0) == op::ISZERO && c(1) == op::PUSH2 && c(4) == op::JUMPI {
                fused[cur] = op::JUMPIFZERO;
                for off in [1, 4] {
                    fused[cur + off] = op::NOP;
                }
                i += 4;
                continue;
            }
            // (3)
            if c(0) == op::DUP2 && c(1) == op::MSTORE && c(2) == op::PUSH1 && c(4) == op::ADD {
                fused[cur] = op::DUP2MSTOREPUSH1ADD;
                for off in [1, 2, 4] {
                    fused[cur + off] = op::NOP;
                }
                i += 4;
                continue;
            }
        }

        // ----------------------------
        // 4-byte 融合
        // SWAP2 SWAP1 POP JUMP  ➜ SWAP2SWAP1POPJUMP
        // SWAP1 POP SWAP2 SWAP1 ➜ SWAP1POPSWAP2SWAP1
        // POP SWAP2 SWAP1 POP   ➜ POPSWAP2SWAP1POP
        // PUSH2 _ _ JUMP        ➜ PUSH2JUMP
        // PUSH2 _ _ JUMPI       ➜ PUSH2JUMPI
        // PUSH1 _ PUSH1         ➜ PUSH1PUSH1
        // ISZERO PUSH2 _ _      ➜ ISZEROPUSH2
        // ----------------------------
        if cur + 3 < fused.len() {
            let c = |o: usize| fused[cur + o];
            // (SWAP2 SWAP1 POP JUMP)
            if c(0) == op::SWAP2 && c(1) == op::SWAP1 && c(2) == op::POP && c(3) == op::JUMP {
                fused[cur] = op::SWAP2SWAP1POPJUMP;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::NOP;
                }
                i += 3;
                continue;
            }
            // (SWAP1 POP SWAP2 SWAP1)
            if c(0) == op::SWAP1 && c(1) == op::POP && c(2) == op::SWAP2 && c(3) == op::SWAP1 {
                fused[cur] = op::SWAP1POPSWAP2SWAP1;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::NOP;
                }
                i += 3;
                continue;
            }
            // (POP SWAP2 SWAP1 POP)
            if c(0) == op::POP && c(1) == op::SWAP2 && c(2) == op::SWAP1 && c(3) == op::POP {
                fused[cur] = op::POPSWAP2SWAP1POP;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::NOP;
                }
                i += 3;
                continue;
            }
            // (PUSH2 .. .. JUMP)
            if c(0) == op::PUSH2 && c(3) == op::JUMP {
                fused[cur] = op::PUSH2JUMP;
                fused[cur + 3] = op::NOP;
                i += 3;
                continue;
            }
            // (PUSH2 .. .. JUMPI)
            if c(0) == op::PUSH2 && c(3) == op::JUMPI {
                fused[cur] = op::PUSH2JUMPI;
                fused[cur + 3] = op::NOP;
                i += 3;
                continue;
            }
            // (PUSH1 _ PUSH1)
            if c(0) == op::PUSH1 && c(2) == op::PUSH1 {
                fused[cur] = op::PUSH1PUSH1;
                fused[cur + 2] = op::NOP;
                i += 3;
                continue;
            }
            // (ISZERO PUSH2 .. ..)
            if c(0) == op::ISZERO && c(1) == op::PUSH2 {
                fused[cur] = op::ISZEROPUSH2;
                fused[cur + 1] = op::NOP;
                i += 3;
                continue;
            }
        }

        // ----------------------------
        // 3-byte 融合
        // PUSH1 _ ADD  ➜ PUSH1ADD
        // PUSH1 _ SHL  ➜ PUSH1SHL
        // PUSH1 _ DUP1 ➜ PUSH1DUP1
        // ----------------------------
        if cur + 2 < fused.len() {
            let inst0 = fused[cur];
            let inst2 = fused[cur + 2];
            if inst0 == op::PUSH1 {
                if inst2 == op::ADD {
                    fused[cur] = op::PUSH1ADD;
                    fused[cur + 2] = op::NOP;
                    i += 2;
                    continue;
                }
                if inst2 == op::SHL {
                    fused[cur] = op::PUSH1SHL;
                    fused[cur + 2] = op::NOP;
                    i += 2;
                    continue;
                }
                if inst2 == op::DUP1 {
                    fused[cur] = op::PUSH1DUP1;
                    fused[cur + 2] = op::NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // ----------------------------
        // 2-byte 融合
        // SWAP1 POP        ➜ SWAP1POP
        // POP JUMP         ➜ POPJUMP
        // POP POP          ➜ POP2
        // SWAP2 SWAP1      ➜ SWAP2SWAP1
        // SWAP2 POP        ➜ SWAP2POP
        // DUP2 LT          ➜ DUP2LT
        // ----------------------------
        if cur + 1 < fused.len() {
            let inst0 = fused[cur];
            let inst1 = fused[cur + 1];
            if inst0 == op::SWAP1 && inst1 == op::POP {
                fused[cur] = op::SWAP1POP;
                fused[cur + 1] = op::NOP;
                i += 1;
                continue;
            }
            if inst0 == op::POP && inst1 == op::JUMP {
                fused[cur] = op::POPJUMP;
                fused[cur + 1] = op::NOP;
                i += 1;
                continue;
            }
            if inst0 == op::POP && inst1 == op::POP {
                fused[cur] = op::POP2;
                fused[cur + 1] = op::NOP;
                i += 1;
                continue;
            }
            if inst0 == op::SWAP2 && inst1 == op::SWAP1 {
                fused[cur] = op::SWAP2SWAP1;
                fused[cur + 1] = op::NOP;
                i += 1;
                continue;
            }
            if inst0 == op::SWAP2 && inst1 == op::POP {
                fused[cur] = op::SWAP2POP;
                fused[cur + 1] = op::NOP;
                i += 1;
                continue;
            }
            if inst0 == op::DUP2 && inst1 == op::LT {
                fused[cur] = op::DUP2LT;
                fused[cur + 1] = op::NOP;
                i += 1;
                continue;
            }
        }

        // ----------------------------
        // 默认：根据 opcode 类型跳过对应的立即数字节
        // ----------------------------
        if let Some(skip) = calculate_skip_steps(&fused, cur) {
            i += skip;
        }
        i += 1;
    }

    Ok(fused)
}

/// 计算在遍历到 `cur` 时需要额外跳过的字节数（不含当前位置）。
fn calculate_skip_steps(code: &[u8], cur: usize) -> Option<usize> {
    let inst = code[cur];

    // 1. 普通 PUSH 指令：跳过立即数长度。
    if inst >= op::PUSH1 && inst <= op::PUSH32 {
        let steps = (inst - op::PUSH1 + 1) as usize;
        return Some(steps);
    }

    // 2. 针对已融合 opcode 的立即数跳过规则。
    match inst {
        op::PUSH2JUMP | op::PUSH2JUMPI => Some(3), // (push2 imm16) + 1 (NOP)
        op::PUSH1PUSH1 => Some(3),                 // push1 imm1 + 1 (NOP)
        op::PUSH1ADD | op::PUSH1SHL | op::PUSH1DUP1 => Some(2),
        op::JUMPIFZERO => Some(4), // PUSH2 imm16 + NOP JUMPI replaced
        _ => None,
    }
}
