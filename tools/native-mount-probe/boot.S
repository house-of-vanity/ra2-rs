.section .multiboot,"a"
.align 4
.long 0x1badb002, 0, -(0x1badb002)
.section .text
.global _start
_start:
 cli
 mov $0x300000, %esp
 mov %cr4, %eax
 or $0x600, %eax
 mov %eax, %cr4
 fninit
 call probe_main
 mov $0xf4, %dx
 mov $0x10, %eax
 out %eax,%dx
1: hlt
 jmp 1b
.section .note.GNU-stack,"",@progbits

.section .text
.global hinge_tail
hinge_tail:
 push %eax
 call capture_hinge
 add $4,%esp
 pop %edi
 pop %esi
 add $0x188,%esp
 ret $4
