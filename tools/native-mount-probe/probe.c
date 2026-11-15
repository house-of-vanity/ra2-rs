/* Developer-only, freestanding i386 probe. No original code is copied here.
 * The original executable is mapped by run.py into a disposable QEMU VM.
 * All floating-point results printed below come from its mounting, hinge and impact functions.
 */
typedef unsigned int u32;
#define THISCALL __attribute__((thiscall))

static void outb(unsigned short port, unsigned char value) {
    __asm__ volatile("outb %0,%1" :: "a"(value), "Nd"(port));
}
static void puts(const char *text) {
    for (; *text; text++) outb(0x3f8, *text);
}
static void hex(u32 value) {
    for (int i = 7; i >= 0; i--)
        outb(0x3f8, "0123456789abcdef"[(value >> (i * 4)) & 15]);
    outb(0x3f8, ' ');
}
static void vec(float *value, int count) {
    for (int i = 0; i < count; i++) {
        union { float f; u32 bits; } word = {.f = value[i]};
        hex(word.bits);
    }
    puts("\n");
}
struct point {
    u32 meta[5];
    float pos[3], q[4];
};
static struct point own = {.q = {0, 0, 0, 1}};
static struct point parent = {.q = {0, 0, 0, 1}};
static u32 vtable[20], component[300];
static float target_q[4], target_anchor[3];

/* Replace scene access and scene writes only. The mount calculation and its
 * quaternion/vector operations remain the original executable's code.
 */
static struct point *THISCALL get(void *self, struct point *out, u32 id) {
    *out = own;
    return out;
}
static void *THISCALL master(void *self) { return self; }
static void THISCALL set_q(void *self, void *body, float *q) {
    for (int i = 0; i < 4; i++) target_q[i] = q[i];
}
static void THISCALL set_anchor(void *self, void *body, float *local, float *world) {
    for (int i = 0; i < 3; i++) target_anchor[i] = local[i];
    puts("ANCHORS\n");
    vec(local, 3);
    vec(world, 3);
}
static void patch(u32 address, void *callback) {
    *(unsigned char *)address = 0xe9;
    *(u32 *)(address + 1) = (u32)callback - address - 5;
}

/* Only body access, allocation-free scene writes and the physics tail are
 * mocked. Original 525830 constructs the reference basis and 4808a0 calculates
 * the absolute orientation. boot.S returns before position/physics updates.
 */
static u32 hinge[100], constraint[100], parent_body[128], child_body[128];
static void THISCALL get_bodies(void *self, u32 *out) {
    out[0] = (u32)parent_body;
    out[1] = (u32)child_body;
}
static void *THISCALL get_body_matrix(void *self) {
    return (unsigned char *)self + 0xcc;
}
void capture_hinge(float *q) {
    puts("HINGE ROTATION WXYZ\n"); vec(q, 4);
}
extern void hinge_tail(void);
static void THISCALL capture_limits(void *self, float start, float end) {
    puts("BURST LIMITS\n"); vec(&start, 1); vec(&end, 1);
}
static void probe_hinges(void) {
    patch(0x525600, get_bodies);
    patch(0x404af0, master);
    patch(0x4a03e0, get_body_matrix);
    patch(0x4809c5, hinge_tail);
    for (int i = 0; i < 3; i++) {
        ((float *)((unsigned char *)parent_body + 0xcc))[i * 4] = 1;
        ((float *)((unsigned char *)child_body + 0xcc))[i * 4] = 1;
    }
    hinge[0x58 / 4] = (u32)constraint;
    hinge[0x5c / 4] = (u32)child_body;
    hinge[0x60 / 4] = (u32)parent_body;
    float pivot[4] = {0, 0, 0, 0};
    float axes[][4] = {
        {1,0,0,0}, {-1,0,0,0}, {0,1,0,0}, {0,-1,0,0},
        {0,0,1,0}, {0,0,-1,0}, {0.8f,0,0.6f,0}, {0.4f,0.8f,0.4472135955f,0},
    };
    typedef void THISCALL (*construct_fn)(void *, float *, float *);
    typedef void THISCALL (*angle_fn)(void *, float);
    for (int index = 0; index < sizeof(axes) / sizeof(axes[0]); index++) {
        ((construct_fn)0x525830)(constraint, pivot, axes[index]);
        puts("HINGE AXIS\n"); vec(axes[index], 3);
        puts("AXIS / PERP IN PARENT\n");
        vec((float *)constraint + 0x50 / 4, 3);
        vec((float *)constraint + 0x70 / 4, 3);
        ((angle_fn)0x4808a0)(hinge, 0);
        ((angle_fn)0x4808a0)(hinge, 1.57079632679f);
        ((angle_fn)0x4808a0)(hinge, 5.03237f);
    }
    patch(0x480800, capture_limits);
    typedef void THISCALL (*stop_fn)(void *, float, u32);
    ((stop_fn)0x480dd0)(hinge, 2.4f, 0);
    ((stop_fn)0x480dd0)(hinge, -2.4f, 0);
}
/* Call the original impact routine without patches to its arithmetic. Bodies
 * are inert data fixtures, so no physics solver or scene allocation is needed.
 * Output: speed, mass A/B, lever, inverse inertia, original impact strength.
 */
static void probe_impacts(void) {
    typedef float THISCALL (*impact_fn)(void *);
    static u32 objects[2][16], bodies[2][128], record[16];
    float cases[][5] = {
        {2,400,0,0,0}, {8,400,0,0,0}, {16,400,0,0,0},
        {8,40,0,0,0}, {8,4000,0,0,0}, {8,400,400,0,0},
        {8,40,40,0,0}, {8,400,0,1,0.02f},
        {8,400,0,1,0.2f}, {8,0,0,0,0},
    };
    for (int k=0;k<sizeof(cases)/sizeof(cases[0]);k++) {
        for (int i=0;i<16;i++) record[i]=0;
        for (int b=0;b<2;b++) {
            for (int i=0;i<128;i++) bodies[b][i]=0;
            objects[b][0x1c/4]=(u32)bodies[b];
            ((float *)bodies[b])[(0x10c+0x28)/4]=cases[k][4];
        }
        record[0x24/4]=(u32)objects[0]; record[0x28/4]=(u32)objects[1];
        ((unsigned char *)record)[0x35]=1;
        ((float *)record)[0]=cases[k][0];
        ((float *)record)[0x08/4]=1; /* normal Y */
        ((float *)record)[0x14/4]=cases[k][3]; /* contact X */
        ((float *)record)[0x2c/4]=cases[k][1];
        ((float *)record)[0x30/4]=cases[k][2];
        float result=((impact_fn)0x45a5a0)(record);
        puts("IMPACT "); vec(cases[k],5); vec(&result,1);
    }
}
/* Only type lookup and the empty scene/decal registry are stand-ins. Original
 * 489470 and 45a5a0 execute all material and impact arithmetic unchanged. */
static u32 material_vtable[4], own_material[4], other_material[4], scene_owner[128];
static u32 THISCALL material_type(void *self) { return ((u32 *)self)[2]; }
static u32 THISCALL type_id(void *self) { return (u32)self == 0x715dc8 ? 2 : 1; }
static void *scene(void) { return scene_owner; }
static void probe_materials(void) {
    patch(0x4cfdf0,type_id);patch(0x401dc0,scene);patch(0x46c020,master);
    material_vtable[2]=(u32)material_type;
    own_material[0]=other_material[0]=(u32)material_vtable;
    ((float *)own_material)[1]=((float *)other_material)[1]=0.1f;
    static u32 source[128], other[128], record[16], objects[2][16], bodies[2][128];
    source[0x64/4]=(u32)own_material;other[0x64/4]=(u32)other_material;
    for (int i=0;i<2;i++) objects[i][0x1c/4]=(u32)bodies[i];
    record[0x24/4]=(u32)objects[0];record[0x28/4]=(u32)objects[1];
    ((unsigned char *)record)[0x35]=1;
    ((float *)record)[0]=8;((float *)record)[0x2c/4]=400;((float *)record)[0x30/4]=400;
    float cases[][4]={{0,1,3,0},{1,0,3,0},{1,1,3,0},{0,1,2,1},{1,0,3,2},{0,1,0,0}};
    typedef float THISCALL (*calculate_fn)(void *,void *,void *);
    for (int i=0;i<sizeof(cases)/sizeof(cases[0]);i++) {
        ((float *)source)[0x68/4]=cases[i][0];((float *)source)[0x6c/4]=cases[i][1];
        own_material[2]=(u32)cases[i][2];other_material[2]=(u32)cases[i][3];
        float result=((calculate_fn)0x489470)(source,other,record);
        puts("MATERIAL ");vec(cases[i],4);vec(&result,1);
    }
}
void probe_main(void) {
    puts("MATH START\n");
    /* Select the executable's own scalar D3DX dispatch table. This avoids
     * Windows CPU-feature discovery; the original arithmetic is unchanged.
     */
    for (int i = 0; i < 0x39; i++)
        ((u32 *)0x6ffcb0)[i] = ((u32 *)0x6ffd98)[i];
    patch(0x45d680, master);
    patch(0x4a86e0, set_q);
    patch(0x4a8a10, set_anchor);
    vtable[0x38 / 4] = (u32)get;
    component[0] = (u32)vtable;
    float position[3] = {0, 0, 0}, rotation[4] = {0, 0, 0, 1};
    typedef void THISCALL (*mount_fn)(void *, u32, float *, float *, float, float, struct point *);
    mount_fn mount = (mount_fn)0x45e510;

    mount(component, 1, position, rotation, 0, 0, &parent);
    puts("MOUNT0\n"); vec(target_q, 4); vec(target_anchor, 3);
    mount(component, 1, position, rotation, 1.57079632679f, 0, &parent);
    puts("MOUNT90\n"); vec(target_q, 4); vec(target_anchor, 3);

    own.pos[0] = 0.3f; own.pos[1] = -0.2f; own.pos[2] = 0.4f;
    own.q[0] = 0.2f; own.q[1] = -0.4f; own.q[2] = 0.1f; own.q[3] = 0.8888194417315589f;
    parent.q[2] = 0.3826834323650898f; parent.q[3] = 0.9238795325112867f;
    position[0] = 1; position[1] = 2; position[2] = 3;
    rotation[0] = 0.25881904510252074f; rotation[3] = 0.9659258262890683f;
    mount(component, 1, position, rotation, 0.7f, 0, &parent);
    puts("MOUNT ARBITRARY\n"); vec(target_q, 4); vec(target_anchor, 3);

    rotation[0] = 0; rotation[3] = 1;
    mount(component, 1, position, rotation, 0.7f, 0, &parent);
    puts("MOUNT SOCKET\n"); vec(target_q, 4);
    probe_hinges();
    probe_impacts();
    probe_materials();
    puts("MATH END\n");
}
