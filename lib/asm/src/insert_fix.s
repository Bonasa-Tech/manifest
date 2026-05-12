# sBPF Assembly: Insert Fix for Red-Black Tree
#
# Function: u32 insert_fix_asm(u8* data, u32 index, u32* root_ptr)
# Args: r1=data, r2=index, r3=root_ptr
# Returns: r0 = next index to fix, or NIL if complete

.globl insert_fix_asm
insert_fix_asm:
    stxdw [r10 - 8], r6
    stxdw [r10 - 16], r7
    stxdw [r10 - 24], r8
    stxdw [r10 - 32], r9

    stxdw [r10 - 40], r1
    stxdw [r10 - 48], r2
    stxdw [r10 - 56], r3
    lddw r6, 4294967295

    ldxw r4, [r3 + 0]
    jne r4, r2, .Linsert_not_root
    mov64 r4, r1
    add64 r4, r2
    mov64 r5, 0
    stxb [r4 + 12], r5
    ja .Linsert_return_nil

.Linsert_not_root:
    mov64 r4, r1
    add64 r4, r2
    ldxw r7, [r4 + 8]
    stxdw [r10 - 64], r7
    mov64 r4, r1
    add64 r4, r7
    ldxb r5, [r4 + 12]
    mov64 r9, 0
    jeq r5, r9, .Linsert_return_nil

    ldxw r8, [r4 + 8]
    stxdw [r10 - 72], r8
    jeq r8, r6, .Linsert_make_parent_black

    ldxdw r1, [r10 - 40]
    mov64 r4, r1
    add64 r4, r8
    ldxw r5, [r4 + 0]
    jne r5, r7, .Linsert_parent_is_right
    mov64 r9, 1
    ldxw r5, [r4 + 4]
    ja .Linsert_have_uncle
.Linsert_parent_is_right:
    mov64 r9, 0
.Linsert_have_uncle:
    stxdw [r10 - 80], r9
    jeq r5, r6, .Linsert_uncle_black
    mov64 r4, r1
    add64 r4, r5
    ldxb r4, [r4 + 12]
    mov64 r9, 1
    jne r4, r9, .Linsert_uncle_black

    ldxdw r7, [r10 - 64]
    mov64 r4, r1
    add64 r4, r7
    mov64 r9, 0
    stxb [r4 + 12], r9
    mov64 r4, r1
    add64 r4, r5
    stxb [r4 + 12], r9
    ldxdw r8, [r10 - 72]
    mov64 r4, r1
    add64 r4, r8
    mov64 r9, 1
    stxb [r4 + 12], r9
    mov64 r0, r8
    ja .Linsert_restore_and_exit

.Linsert_uncle_black:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 48]
    ldxdw r7, [r10 - 64]
    ldxdw r8, [r10 - 72]
    mov64 r4, r1
    add64 r4, r8
    ldxb r5, [r4 + 12]
    stxdw [r10 - 88], r5
    mov64 r4, r1
    add64 r4, r2
    ldxb r5, [r4 + 12]
    stxdw [r10 - 96], r5
    mov64 r4, r1
    add64 r4, r7
    ldxw r5, [r4 + 0]
    jne r5, r2, .Linsert_current_is_right
    mov64 r9, 1
    ja .Linsert_have_current_side
.Linsert_current_is_right:
    mov64 r9, 0
.Linsert_have_current_side:
    stxdw [r10 - 104], r9

    ldxdw r4, [r10 - 80]
    jeq r4, 0, .Linsert_parent_right_cases
    ldxdw r5, [r10 - 104]
    jeq r5, 0, .Linsert_case_left_right

.Linsert_case_left_left:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 72]
    ldxdw r3, [r10 - 56]
    call rotate_right_asm

    ldxdw r1, [r10 - 40]
    ldxdw r8, [r10 - 72]
    mov64 r4, r1
    add64 r4, r8
    mov64 r5, 1
    stxb [r4 + 12], r5
    ldxdw r7, [r10 - 64]
    mov64 r4, r1
    add64 r4, r7
    ldxdw r5, [r10 - 88]
    stxb [r4 + 12], r5
    ja .Linsert_return_nil

.Linsert_case_left_right:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 64]
    ldxdw r3, [r10 - 56]
    call rotate_left_asm
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 72]
    ldxdw r3, [r10 - 56]
    call rotate_right_asm

    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 48]
    mov64 r4, r1
    add64 r4, r2
    ldxdw r5, [r10 - 88]
    stxb [r4 + 12], r5
    ldxdw r8, [r10 - 72]
    mov64 r4, r1
    add64 r4, r8
    ldxdw r5, [r10 - 96]
    stxb [r4 + 12], r5
    ja .Linsert_return_nil

.Linsert_parent_right_cases:
    ldxdw r5, [r10 - 104]
    jne r5, 0, .Linsert_case_right_left

.Linsert_case_right_right:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 72]
    ldxdw r3, [r10 - 56]
    call rotate_left_asm

    ldxdw r1, [r10 - 40]
    ldxdw r8, [r10 - 72]
    mov64 r4, r1
    add64 r4, r8
    mov64 r5, 1
    stxb [r4 + 12], r5
    ldxdw r7, [r10 - 64]
    mov64 r4, r1
    add64 r4, r7
    ldxdw r5, [r10 - 88]
    stxb [r4 + 12], r5
    ja .Linsert_return_nil

.Linsert_case_right_left:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 64]
    ldxdw r3, [r10 - 56]
    call rotate_right_asm
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 72]
    ldxdw r3, [r10 - 56]
    call rotate_left_asm

    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 48]
    mov64 r4, r1
    add64 r4, r2
    ldxdw r5, [r10 - 88]
    stxb [r4 + 12], r5
    ldxdw r8, [r10 - 72]
    mov64 r4, r1
    add64 r4, r8
    ldxdw r5, [r10 - 96]
    stxb [r4 + 12], r5
    ja .Linsert_return_nil

.Linsert_make_parent_black:
    mov64 r5, 0
    stxb [r4 + 12], r5

.Linsert_return_nil:
    lddw r0, 4294967295

.Linsert_restore_and_exit:
    ldxdw r6, [r10 - 8]
    ldxdw r7, [r10 - 16]
    ldxdw r8, [r10 - 24]
    ldxdw r9, [r10 - 32]
    exit
