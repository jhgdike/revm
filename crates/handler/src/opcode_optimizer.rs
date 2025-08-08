use thiserror::Error;
use bytecode::opcode as op;

/// superinstruction in revm

/// 自定义优化 opcode 的最小与最大取值范围。
pub(crate) const MIN_OPTIMIZED_OPCODE: u8 = 0xB0;
/// superinstruction max opcode
pub(crate) const MAX_OPTIMIZED_OPCODE: u8 = 0xD0;

/// FailPreprocessing Fusion err
#[derive(Debug, Error)]
pub(crate) enum FusionError {
    #[error("optimized opcode already present (pre-processing fail)")]
    /// Input bytecode already contains optimized opcodes; fusion aborted.
    FailPreprocessing,
}

/// 对字节码进行模式匹配融合，生成新的字节码副本。
///
/// 1. `code` 本身保持不变，返回新的 `Vec<u8>`；
/// 2. 若发现字节码中已包含任何优化 opcode（0xB0~0xC8），直接返回 `FusionError::FailPreprocessing`；
/// 3. 若在遍历过程中遇到 `INVALID`(0xFE) 则提早终止并返回当前结果。
pub(crate) fn do_code_fusion(code: &[u8]) -> Result<Vec<u8>, FusionError> {
    // return Ok(code.to_vec());
    let mut fused = code.to_vec();
    let mut i = 0usize;
    while i < fused.len() {
        let cur = i;

        // 1. 提前终止：若遇到 INVALID。
        if fused[cur] == op::INVALID {
            return Ok(fused);
        }

        // 2. 预处理：若已包含优化 opcode，直接报错。
        if fused[cur] <= MIN_OPTIMIZED_OPCODE && fused[cur] >= MAX_OPTIMIZED_OPCODE {
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
                    fused[cur + off] = op::SNOP;
                }
                i += 16;
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
                    fused[cur + off] = op::SNOP;
                }
                i += 13;
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
                    fused[cur + off] = op::SNOP;
                }
                i += 10;
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
                    fused[cur + off] = op::SNOP;
                }
                i += 8;
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
                    fused[cur + off] = op::SNOP;
                }
                i += 6;
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
                    fused[cur + off] = op::SNOP;
                }
                i += 5;
                continue;
            }
            // (2)
            if c(0) == op::ISZERO && c(1) == op::PUSH2 && c(4) == op::JUMPI {
                fused[cur] = op::JUMPIFZERO;
                for off in [1, 4] {
                    fused[cur + off] = op::SNOP;
                }
                i += 5;
                continue;
            }
            // (3)
            if c(0) == op::DUP2 && c(1) == op::MSTORE && c(2) == op::PUSH1 && c(4) == op::ADD {
                fused[cur] = op::DUP2MSTOREPUSH1ADD;
                for off in [1, 2, 4] {
                    fused[cur + off] = op::SNOP;
                }
                i += 5;
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
                    fused[cur + off] = op::SNOP;
                }
                i += 4;
                continue;
            }
            // (SWAP1 POP SWAP2 SWAP1)
            if c(0) == op::SWAP1 && c(1) == op::POP && c(2) == op::SWAP2 && c(3) == op::SWAP1 {
                fused[cur] = op::SWAP1POPSWAP2SWAP1;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::SNOP;
                }
                i += 4;
                continue;
            }
            // (POP SWAP2 SWAP1 POP)
            if c(0) == op::POP && c(1) == op::SWAP2 && c(2) == op::SWAP1 && c(3) == op::POP {
                fused[cur] = op::POPSWAP2SWAP1POP;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::SNOP;
                }
                i += 4;
                continue;
            }
            // (PUSH2 .. .. JUMP)
            if c(0) == op::PUSH2 && c(3) == op::JUMP {
                fused[cur] = op::PUSH2JUMP;
                fused[cur + 3] = op::SNOP;
                i += 4;
                continue;
            }
            // (PUSH2 .. .. JUMPI)
            if c(0) == op::PUSH2 && c(3) == op::JUMPI {
                fused[cur] = op::PUSH2JUMPI;
                fused[cur + 3] = op::SNOP;
                i += 4;
                continue;
            }
            // (PUSH1 _ PUSH1)
            if c(0) == op::PUSH1 && c(2) == op::PUSH1 {
                fused[cur] = op::PUSH1PUSH1;
                fused[cur + 2] = op::SNOP;
                i += 4;
                continue;
            }
            // (ISZERO PUSH2 .. ..)
            if c(0) == op::ISZERO && c(1) == op::PUSH2 {
                fused[cur] = op::ISZEROPUSH2;
                fused[cur + 1] = op::SNOP;
                i += 4;
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
                    fused[cur + 2] = op::SNOP;
                    i += 3;
                    continue;
                }
                if inst2 == op::SHL {
                    fused[cur] = op::PUSH1SHL;
                    fused[cur + 2] = op::SNOP;
                    i += 3;
                    continue;
                }
                if inst2 == op::DUP1 {
                    fused[cur] = op::PUSH1DUP1;
                    fused[cur + 2] = op::SNOP;
                    i += 3;
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
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
            if inst0 == op::POP && inst1 == op::JUMP {
                fused[cur] = op::POPJUMP;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
            if inst0 == op::POP && inst1 == op::POP {
                fused[cur] = op::POP2;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
            if inst0 == op::SWAP2 && inst1 == op::SWAP1 {
                fused[cur] = op::SWAP2SWAP1;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
            if inst0 == op::SWAP2 && inst1 == op::POP {
                fused[cur] = op::SWAP2POP;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
            if inst0 == op::DUP2 && inst1 == op::LT {
                fused[cur] = op::DUP2LT;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
        }

        // ----------------------------
        // New fused patterns from Go example
        // ----------------------------
        
        // DUP3 AND (2 bytes)
        if cur + 1 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::DUP3 && c(1) == op::AND {
                fused[cur] = op::DUP3AND;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
        }

        // SWAP2 SWAP1 DUP3 SUB SWAP2 DUP3 GT PUSH2 (9 bytes + 2 immediate bytes)
        if cur + 10 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::SWAP2 && c(1) == op::SWAP1 && c(2) == op::DUP3 && c(3) == op::SUB 
                && c(4) == op::SWAP2 && c(5) == op::DUP3 && c(6) == op::GT && c(7) == op::PUSH2 {
                fused[cur] = op::SWAP2SWAP1DUP3SUBSWAP2DUP3GTPUSH2;
                for off in [1, 2, 3, 4, 5, 6, 7] {
                    fused[cur + off] = op::SNOP;
                }
                i += 11; // 7 opcodes + 2 immediate bytes + skip increment
                continue;
            }
        }

        // SWAP1 DUP2 (2 bytes)
        if cur + 1 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::SWAP1 && c(1) == op::DUP2 {
                fused[cur] = op::SWAP1DUP2;
                fused[cur + 1] = op::SNOP;
                i += 2;
                continue;
            }
        }

        // SHR SHR DUP1 MUL DUP1 (5 bytes)
        if cur + 4 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::SHR && c(1) == op::SHR && c(2) == op::DUP1 && c(3) == op::MUL && c(4) == op::DUP1 {
                fused[cur] = op::SHRSHRDUP1MULDUP1;
                for off in [1, 2, 3, 4] {
                    fused[cur + off] = op::SNOP;
                }
                i += 5;
                continue;
            }
        }

        // SWAP3 POP POP POP (4 bytes)
        if cur + 3 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::SWAP3 && c(1) == op::POP && c(2) == op::POP && c(3) == op::POP {
                fused[cur] = op::SWAP3POPPOPPOP;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::SNOP;
                }
                i += 4;
                continue;
            }
        }

        // SUB SLT ISZERO PUSH2 (5 bytes + 2 immediate)
        if cur + 6 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::SUB && c(1) == op::SLT && c(2) == op::ISZERO && c(3) == op::PUSH2 {
                fused[cur] = op::SUBSLTISZEROPUSH2;
                for off in [1, 2, 3] {
                    fused[cur + off] = op::SNOP;
                }
                i += 7; // 4 opcodes + 2 immediate bytes + skip increment
                continue;
            }
        }

        // DUP11 MUL DUP3 SUB MUL DUP1 (6 bytes)
        if cur + 5 < fused.len() {
            let c = |o: usize| fused[cur + o];
            if c(0) == op::DUP11 && c(1) == op::MUL && c(2) == op::DUP3 && c(3) == op::SUB && c(4) == op::MUL && c(5) == op::DUP1 {
                fused[cur] = op::DUP11MULDUP3SUBMULDUP1;
                for off in [1, 2, 3, 4, 5] {
                    fused[cur + off] = op::SNOP;
                }
                i += 6;
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
        // New fused opcodes with immediates
        op::SWAP2SWAP1DUP3SUBSWAP2DUP3GTPUSH2 => Some(3), // includes PUSH2 immediate
        op::SUBSLTISZEROPUSH2 => Some(4), // includes PUSH2 immediate
        _ => None,
    }
}

/// BasicBlock represents a sequence of opcodes that can be executed linearly
/// without any jumps in or out except at the beginning and end.
#[derive(Debug)]
pub(crate) struct BasicBlock {
    /// inclusive start PC
    pub start_pc: usize,
    /// exclusive end PC
    pub end_pc: usize,
    /// raw bytes of opcodes in this block
    pub opcodes: Vec<u8>,
    // /// If this block ends with a jump, the target PC else None
    // pub jump_target: Option<usize>,
    /// Whether this block starts with a JUMPDEST
    pub is_jump_dest: bool,
}

impl BasicBlock {
    /// 将整段字节码切分为若干 BasicBlock
    pub(crate) fn generate(code: &[u8]) -> Box<[Self]> {
        if code.is_empty() {
            return Vec::new().into_boxed_slice();
        }

        use std::collections::HashSet;

        // -------------- First pass: identify all JUMPDEST locations --------------
        let mut jump_dests: HashSet<usize> = HashSet::new();
        let mut pc = 0usize;
        while pc < code.len() {
            let op = code[pc];
            if op == op::JUMPDEST {
                jump_dests.insert(pc);
                pc += 1;
                continue
            }
            // Add 1 for the opcode byte
            if let Some(skip) = calculate_skip_steps(code, pc) {
                pc += 1 + skip;
            } else {
                pc += 1;
            }
        }

        // -------------- Second pass: build basic blocks --------------
        let mut blocks: Vec<BasicBlock> = Vec::new();
        pc = 0;
        let mut current: Option<BasicBlock> = None;
        while pc < code.len() {
            let op = code[pc];

            // 需要开始新块的条件：
            if op == op::INVALID || jump_dests.contains(&pc) {
                if let Some(mut blk) = current.take() {
                    blk.end_pc = pc;
                    blocks.push(blk);
                }
                current = Some(BasicBlock {
                    start_pc: pc,
                    end_pc: 0,
                    opcodes: Vec::new(),
                    // jump_target: None,
                    is_jump_dest: op == op::JUMPDEST,
                });
            } else if current.is_none() {
                current = Some(BasicBlock {
                    start_pc: pc,
                    end_pc: 0,
                    opcodes: Vec::new(),
                    // jump_target: None,
                    is_jump_dest: op == op::JUMPDEST,
                });
            }

            // Determine instruction length
            let (inst_len, _has_immediate) = if let Some(skip) = calculate_skip_steps(code, pc) {
                (1 + skip, true)
            } else {
                (1, false)
            };

            // Check bounds before accessing
            let bounded_len = if pc + inst_len > code.len() {
                code.len() - pc
            } else {
                inst_len
            };

            // Add instruction bytes to block
            if let Some(ref mut blk) = current {
                blk.opcodes.extend_from_slice(&code[pc..pc + bounded_len]);
            }

            pc += bounded_len;

            // If this is a block terminator (other than INVALID since we already handled it), end the block
            if is_block_terminator(op) {
                if let Some(mut blk) = current.take() {
                    blk.end_pc = pc;
                    // 处理无条件跳转目标（JUMP / RJUMP / JUMPF）
                    // if (op == op::JUMP) && has_immediate {
                    //     let imm_start = blk.opcodes.len() - (inst_len - 1); // 跳过 opcode 本身
                    //     let imm_bytes = &blk.opcodes[imm_start..];
                    //     // 截取低 8 字节转 usize
                    //     let mut tgt: usize = 0;
                    //     for &b in imm_bytes.iter().rev().take(8) {
                    //         tgt = (tgt << 8) | b as usize;
                    //     }
                    //     blk.jump_target = Some(tgt);
                    // }
                    blocks.push(blk);
                    current = None;
                }
            }
        }

        if let Some(mut blk) = current {
            blk.end_pc = pc;
            blocks.push(blk);
        }

        blocks.into_boxed_slice()
    }
}

/// 判断给定 opcode 是否终结 BasicBlock
fn is_block_terminator(op: u8) -> bool {
    matches!(
        op,
        op::STOP
            | op::RETURN
            | op::REVERT
            | op::SELFDESTRUCT
            | op::JUMP
            | op::JUMPI
    )
}

// =============================================================================
//  CFG-based opcode fusion – translated from provided Go implementation
// =============================================================================

/// 基于基本块（CFG）分析的 opcode 融合入口。
///
/// * 若字节码为空或基本块产生失败，则返回 `FusionError::FailPreprocessing`；
/// * 如发现任何块中已出现优化 opcode（0xB0–0xC8），立即返回同样错误；
/// * 否则仅对选定类型的基本块执行融合，其余保持原状。
pub(crate) fn do_basic_block_opcode_fusion(code: &[u8]) -> Result<Vec<u8>, FusionError> {
    // for byte in code {
    //     if *byte >= MIN_OPTIMIZED_OPCODE && *byte < MAX_OPTIMIZED_OPCODE {
    //         print!("{:?}", byte);
    //         return Err(FusionError::FailPreprocessing);
    //     }
    // }
    // if (MIN_OPTIMIZED_OPCODE..=MAX_OPTIMIZED_OPCODE).contains(&byte) {
    //     return Err(FusionError::FailPreprocessing);
    // }
    // 生成基本块
    let blocks = BasicBlock::generate(code);
    if blocks.is_empty() {
        return Err(FusionError::FailPreprocessing);
    }

    // 拷贝原始字节码，后续修改写回此副本
    let mut fused_code = code.to_vec();

    // 遍历每个基本块
    for (idx, block) in blocks.iter().enumerate() {
        // 跳过类型为 Others 的块
        let blk_ty = get_block_type(block, &blocks, idx);
        if matches!(blk_ty, BlockType::Others) {
            continue;
        }

        // print!("{:?} - {:?}\n", block.start_pc, block.end_pc);
        // ---------- 预扫描：检测优化 opcode ----------
        let mut pc = block.start_pc;
        while pc < block.end_pc && pc < code.len() {
            let byte = code[pc];
            if (MIN_OPTIMIZED_OPCODE..=MAX_OPTIMIZED_OPCODE).contains(&byte) {
                return Err(FusionError::FailPreprocessing);
            }
            if let Some(skip) = calculate_skip_steps(code, pc) {
                pc += 1 + skip;
            } else {
                pc += 1;
            }
        }

        // ---------- 检测 INVALID ----------
        let mut pc = block.start_pc;
        let mut has_invalid = false;
        while pc < block.end_pc && pc < code.len() {
            if code[pc] == op::INVALID {
                has_invalid = true;
                break;
            }
            if let Some(skip) = calculate_skip_steps(code, pc) {
                pc += 1 + skip;
            } else {
                pc += 1;
            }
        }
        if has_invalid {
            continue; // 跳过含 INVALID 的块
        }

        // ---------- 应用融合 ----------
        fuse_block(&mut fused_code, block)?;
    }

    Ok(fused_code)
}

// -----------------------------------------------------------------------------
//  Block-level helpers
// -----------------------------------------------------------------------------

/// 区分基本块类型（与 Go 版本保持一致）
#[derive(PartialEq, Eq)]
enum BlockType {
    Empty,
    EntryBB,
    JumpDest,
    ConditionalFallthrough,
    Others,
}

fn get_block_type(block: &BasicBlock, blocks: &[BasicBlock], index: usize) -> BlockType {
    if block.opcodes.is_empty() {
        return BlockType::Empty;
    }
    if block.start_pc == 0 {
        return BlockType::EntryBB;
    }
    if block.is_jump_dest {
        return BlockType::JumpDest;
    }
    if index > 0 {
        let prev = &blocks[index - 1];
        if let Some(&last) = prev.opcodes.last() {
            if last == op::JUMPI {
                return BlockType::ConditionalFallthrough;
            }
        }
    }
    BlockType::Others
}

/// 对单个基本块执行 opcode 融合。直接在 `code` 切片上原地修改。
fn fuse_block(code: &mut [u8], block: &BasicBlock) -> Result<(), FusionError> {
    let start = block.start_pc;
    let end = block.end_pc.min(code.len());
    if start >= end {
        return Ok(());
    }

    // 为了复用已有的 `do_code_fusion` 逻辑，我们对块切片执行一次整体融合，
    // 之后把结果写回原字节码。
    {
        let slice = &code[start..end];
        let fused_slice = do_code_fusion(slice)?; // 可能返回 FailPreprocessing，但前面已排除。
        debug_assert_eq!(fused_slice.len(), slice.len());
        // SAFETY: start..end 与 fused_slice 长度一致
        code[start..end].copy_from_slice(&fused_slice);
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use bytecode::Bytecode;
    use std::fs;
    use primitives::{hex::{self, ToHexExt}, Bytes};
    use crate::opcode_optimizer::is_block_terminator;

    #[test]
    fn test_is_block_terminator() {
        assert_eq!(is_block_terminator(op::STOP), true);
        assert_eq!(is_block_terminator(op::RETURN), true);
        assert_eq!(is_block_terminator(op::REVERT), true);
        assert_eq!(is_block_terminator(op::SELFDESTRUCT), true);
        assert_eq!(is_block_terminator(op::JUMP), true);
        assert_eq!(is_block_terminator(op::JUMPI), true);

        assert_ne!(is_block_terminator(op::ADD), true);
        assert_ne!(is_block_terminator(op::GT), true);
        assert_ne!(is_block_terminator(op::ADDMOD), true);
        assert_ne!(is_block_terminator(op::LT), true);
        assert_ne!(is_block_terminator(op::PUSH1), true);
    }

    #[test]
    fn test_do_fusion() {
        let code = load_bytecode("/Users/wangtao/git_repo/revm_task/benchmark_test/bytecode/BIGA.bin");
        // println!("{:?}", Bytecode::new_raw(Bytes::from(code.clone())).legacy_jump_table().unwrap().as_slice());
        match do_basic_block_opcode_fusion(code.as_ref()) {
            // Ok(bytecode) => println!("{:?}", Bytecode::new_raw(Bytes::from(bytecode.clone())).legacy_jump_table().unwrap().as_slice()),
            Ok(bytecode) => println!("{:?}", bytecode.encode_hex()),
            Err(e) => panic!("{:?}", e),
        }

        match do_code_fusion(code.as_ref()) {
            Ok(bytecode) => println!("{:?}", bytecode.encode_hex()),
            Err(e) => panic!("{:?}", e),
        }
    }

    fn load_bytecode(path: &str) -> Bytes {
        let bytecode_str = fs::read_to_string(path)
            .expect("Failed to read bytecode file");
        let bytecode_str = bytecode_str.trim();
    
        if bytecode_str.starts_with("0x") {
            hex::decode(&bytecode_str[2..]).expect("Invalid hex in bytecode").into()
        } else {
            hex::decode(bytecode_str).expect("Invalid hex in bytecode").into()
        }
    }
}
