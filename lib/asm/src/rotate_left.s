# sBPF Assembly: Optimized Left Rotation for Red-Black Tree
#
# Function: void rotate_left_asm(u8* data, u32 g_index, u32* root_ptr)
# Args: r1=data, r2=g_index, r3=root_ptr

.globl rotate_left_asm
rotate_left_asm:
    stxdw [r10 - 8], r6
    stxdw [r10 - 16], r7
    stxdw [r10 - 24], r8
    stxdw [r10 - 32], r9
    lddw r0, 4294967295
    mov64 r6, r1
    add64 r6, r2
    ldxw r7, [r6 + 4]
    ldxw r9, [r6 + 8]
    mov64 r4, r1
    add64 r4, r7
    ldxw r8, [r4 + 0]

    stxw [r4 + 8], r9
    stxw [r4 + 0], r2

    stxw [r6 + 8], r7
    stxw [r6 + 4], r8
    jeq r8, r0, .Lrotate_left_update_gg
    mov64 r5, r1
    add64 r5, r8
    stxw [r5 + 8], r2

.Lrotate_left_update_gg:
    jeq r9, r0, .Lrotate_left_update_root
    mov64 r5, r1
    add64 r5, r9
    ldxw r4, [r5 + 0]
    jne r4, r2, .Lrotate_left_try_gg_right
    stxw [r5 + 0], r7
    ja .Lrotate_left_update_root
.Lrotate_left_try_gg_right:
    ldxw r4, [r5 + 4]
    jne r4, r2, .Lrotate_left_update_root
    stxw [r5 + 4], r7

.Lrotate_left_update_root:
    ldxw r4, [r3 + 0]
    jne r4, r2, .Lrotate_left_done
    stxw [r3 + 0], r7

.Lrotate_left_done:
    ldxdw r6, [r10 - 8]
    ldxdw r7, [r10 - 16]
    ldxdw r8, [r10 - 24]
    ldxdw r9, [r10 - 32]
    exit
