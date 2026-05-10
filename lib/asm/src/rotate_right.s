; sBPF Assembly: Optimized Right Rotation for Red-Black Tree
;
; Right rotate of G:
;         GG                     GG
;         |                      |
;         G                      P
;       /   \                  /   \
;      P     U     --->      X       G
;    /  \                          /   \
;  X     Y                       Y       U
;
; Function: void rotate_right_asm(u8* data, u32 g_index, u32* root_ptr)
; Args: r1=data, r2=g_index, r3=root_ptr
;
; OPTIMIZATIONS:
; - Single NIL constant load at start
; - Batched memory reads before writes
; - Eliminated redundant pointer recalculations
; - Total: ~25 instructions (down from ~35)

.equ NODE_LEFT,   0x00
.equ NODE_RIGHT,  0x04
.equ NODE_PARENT, 0x08
.equ NIL, 0xFFFFFFFF

.globl rotate_right_asm
rotate_right_asm:
    ; r0 = NIL (keep constant in register throughout)
    mov64 r0, NIL

    ; Calculate g_ptr and load all needed fields at once
    add64 r6, r1, r2                    ; r6 = g_ptr = data + g_index
    ldxw r7, [r6+NODE_LEFT]             ; r7 = p_index = g->left
    ldxw r9, [r6+NODE_PARENT]           ; r9 = gg_index = g->parent

    ; Calculate p_ptr and load y_index
    add64 r4, r1, r7                    ; r4 = p_ptr = data + p_index
    ldxw r8, [r4+NODE_RIGHT]            ; r8 = y_index = p->right

    ; === Batch all writes ===
    ; Update P node (p->parent = gg, p->right = g)
    stxw [r4+NODE_PARENT], r9
    stxw [r4+NODE_RIGHT], r2

    ; Update G node (g->parent = p, g->left = y)
    stxw [r6+NODE_PARENT], r7
    stxw [r6+NODE_LEFT], r8

    ; Update Y->parent if Y != NIL
    jeq r8, r0, update_gg
    add64 r5, r1, r8                    ; r5 = y_ptr
    stxw [r5+NODE_PARENT], r2           ; y->parent = g_index

update_gg:
    ; Update GG child pointer if GG != NIL
    jeq r9, r0, update_root
    add64 r5, r1, r9                    ; r5 = gg_ptr
    ldxw r4, [r5+NODE_LEFT]
    jne r4, r2, try_gg_right
    stxw [r5+NODE_LEFT], r7             ; gg->left = p_index
    ja update_root
try_gg_right:
    ldxw r4, [r5+NODE_RIGHT]
    jne r4, r2, update_root
    stxw [r5+NODE_RIGHT], r7            ; gg->right = p_index

update_root:
    ; Update root if G was root
    ldxw r4, [r3+0]
    jne r4, r2, done
    stxw [r3+0], r7                     ; *root_ptr = p_index

done:
    exit
