#include <stdint.h>
#include <stddef.h>

/* UART0 — already configured at 115200 by the bootloader */
#define UART0_BASE          0x60000000u
#define UART_FIFO_REG       (*(volatile uint32_t *)(UART0_BASE + 0x00u))
#define UART_STATUS_REG     (*(volatile uint32_t *)(UART0_BASE + 0x1Cu))
/* STATUS bits */
#define UART_RXFIFO_CNT(s)  ((s) & 0xFFu)
#define UART_TXFIFO_CNT(s)  (((s) >> 16) & 0xFFu)
#define UART_TXFIFO_MAX     126u   /* leave 2-byte margin in 128-byte FIFO */

extern char _image_end;

/* ---- output ---------------------------------------------------------- */

static void putc_uart(char c)
{
    while (UART_TXFIFO_CNT(UART_STATUS_REG) >= UART_TXFIFO_MAX) {}
    UART_FIFO_REG = (uint8_t)c;
}

static void puts_uart(const char *s)
{
    while (*s) {
        if (*s == '\n') putc_uart('\r');
        putc_uart(*s++);
    }
}

static void put_hex32(uint32_t v)
{
    static const char hex[] = "0123456789abcdef";
    puts_uart("0x");
    for (int i = 28; i >= 0; i -= 4)
        putc_uart(hex[(v >> i) & 0xfu]);
}

static void put_dec(uint32_t v)
{
    char buf[10];
    int i = 0;
    if (v == 0) { putc_uart('0'); return; }
    while (v && i < (int)sizeof(buf)) { buf[i++] = '0' + (char)(v % 10u); v /= 10u; }
    while (i > 0) putc_uart(buf[--i]);
}

static void print_kv_hex(const char *k, uint32_t v)
{
    puts_uart(k); puts_uart(": "); put_hex32(v); puts_uart("\n");
}

/* ---- input ----------------------------------------------------------- */

static int getc_uart(void)
{
    if (UART_RXFIFO_CNT(UART_STATUS_REG) == 0) return -1;
    return (int)(UART_FIFO_REG & 0xFFu);
}

static int getc_uart_blocking(void)
{
    int c;
    while ((c = getc_uart()) < 0) {}
    return c;
}

/* ---- CSR helpers ----------------------------------------------------- */

static uint32_t csr_mhartid(void)
{
    uint32_t v; __asm__ volatile("csrr %0, mhartid" : "=r"(v)); return v;
}
static uint32_t csr_mstatus(void)
{
    uint32_t v; __asm__ volatile("csrr %0, mstatus" : "=r"(v)); return v;
}
static uint32_t csr_mtvec(void)
{
    uint32_t v; __asm__ volatile("csrr %0, mtvec"  : "=r"(v)); return v;
}
/* ESP32-C3 SYSTIMER unit 0 — latch then read 32-bit low value */
#define SYSTIMER_BASE           0x60023000u
#define SYSTIMER_UNIT0_OP_REG   (*(volatile uint32_t *)(SYSTIMER_BASE + 0x04u))
#define SYSTIMER_UNIT0_VALUE_LO (*(volatile uint32_t *)(SYSTIMER_BASE + 0x40u))
#define SYSTIMER_OP_UPDATE      (1u << 30)

static uint32_t sys_ticks(void)
{
    SYSTIMER_UNIT0_OP_REG = SYSTIMER_OP_UPDATE;
    return SYSTIMER_UNIT0_VALUE_LO;
}
static uint32_t current_sp(void)
{
    uint32_t v; __asm__ volatile("mv %0, sp" : "=r"(v)); return v;
}

/* ---- OTA flash helpers ----------------------------------------------- */

#define OTA0_OFFSET      0x20000u
#define OTA0_SIZE        0x1E000u  /* 120 KB */
#define OTA_DATA_OFFSET  0x3E000u
#define SECTOR_SIZE      4096u

/* ROM function pointers */
typedef int  (*rom_spi_unlock_t)(void);
typedef int  (*rom_spi_erase_t)(uint32_t sector_num);
typedef int  (*rom_spi_write_t)(uint32_t dest_addr, const void *src, int32_t len);
typedef void (*rom_reset_t)(void) __attribute__((noreturn));

#define ROM_UNLOCK  ((rom_spi_unlock_t)0x40000140u)
#define ROM_ERASE   ((rom_spi_erase_t) 0x40000128u)
#define ROM_WRITE   ((rom_spi_write_t) 0x4000012cu)
#define ROM_RESET   ((rom_reset_t)     0x40000090u)

/* CRC32-LE matching bootloader_crc32_le (no final XOR, poly 0xEDB88320) */
static uint32_t crc32_le(uint32_t crc, const uint8_t *data, uint32_t len)
{
    for (uint32_t i = 0; i < len; i++) {
        crc ^= data[i];
        for (int b = 0; b < 8; b++)
            crc = (crc & 1) ? (crc >> 1) ^ 0xEDB88320u : (crc >> 1);
    }
    return crc;
}

/* OTA-data entry: {ota_seq, ota_state, crc} — 12 bytes at the start of a sector */
#define OTA_STATE_NEW     0u
#define OTA_STATE_VALID   2u

static void write_ota_data_entry(uint32_t sector_addr, uint32_t seq, uint32_t state)
{
    uint32_t entry[4];   /* 16 bytes — must be word-aligned; ROM writes in words */
    entry[0] = seq;
    entry[1] = state;
    entry[2] = crc32_le(0xFFFFFFFFu, (const uint8_t *)&entry[0], 4);
    entry[3] = 0;

    uint32_t sector_num = sector_addr / SECTOR_SIZE;
    ROM_UNLOCK();
    ROM_ERASE(sector_num);
    ROM_WRITE(sector_addr, entry, 16);
}

/* ---- v2: mark this boot valid so rollback doesn't trigger ------------ */

#ifdef V2
static void mark_ota_valid(void)
{
    /* Write VALID to sector 0 of otadata (cancels the PENDING_VERIFY that   *
     * the bootloader wrote to sector 1 before jumping to us).               */
    write_ota_data_entry(OTA_DATA_OFFSET, 1, OTA_STATE_VALID);
}
#endif

/* ---- v1: OTA receive command ----------------------------------------- */

static uint32_t parse_dec(const char *s)
{
    uint32_t v = 0;
    while (*s >= '0' && *s <= '9')
        v = v * 10u + (uint32_t)(*s++ - '0');
    return v;
}

#ifndef V2
static void cmd_ota(const char *args)
{
    uint32_t size = parse_dec(args);
    if (size == 0 || size > OTA0_SIZE) {
        puts_uart("ERR: bad size\n");
        return;
    }

    /* Drain any residual UART bytes (e.g. the LF that follows the CR in
     * "ota <n>\r\n") before starting the binary receive stream. */
    for (volatile uint32_t i = 0; i < 5000u; i++) {}
    while (getc_uart() >= 0) {}

    puts_uart("READY\r\n");

    /* Erase enough sectors in ota_0 to hold the new firmware */
    uint32_t sectors = (size + SECTOR_SIZE - 1) / SECTOR_SIZE;
    ROM_UNLOCK();
    for (uint32_t i = 0; i < sectors; i++)
        ROM_ERASE(OTA0_OFFSET / SECTOR_SIZE + i);

    /* Receive firmware in 256-byte chunks, write each to flash */
    uint32_t written = 0;
    while (written < size) {
        /* Stack buffer — 4-byte aligned for ROM_WRITE */
        uint32_t buf_words[64];  /* 256 bytes */
        uint8_t *buf = (uint8_t *)buf_words;

        uint32_t chunk = size - written;
        if (chunk > 256u) chunk = 256u;

        /* Receive chunk bytes (blocking) */
        for (uint32_t i = 0; i < chunk; i++)
            buf[i] = (uint8_t)getc_uart_blocking();

        /* Zero-pad to multiple of 4 for ROM_WRITE */
        uint32_t aligned = (chunk + 3u) & ~3u;
        for (uint32_t i = chunk; i < aligned; i++)
            buf[i] = 0;

        ROM_WRITE(OTA0_OFFSET + written, buf_words, (int32_t)aligned);
        written += chunk;

        putc_uart('.');  /* progress ACK to host */
    }

    /* Write OTA data: sector 0 gets seq=1, state=NEW */
    write_ota_data_entry(OTA_DATA_OFFSET, 1, OTA_STATE_NEW);

    puts_uart("\r\nOK\r\n");

    /* Brief pause so the host can read the OK before serial disappears */
    for (volatile uint32_t i = 0; i < 200000u; i++) {}

    ROM_RESET();
}
#endif  /* !V2 */

/* ---- commands -------------------------------------------------------- */

static int streq(const char *a, const char *b)
{
    while (*a && *a == *b) { a++; b++; }
    return *a == *b;
}
static int startswith(const char *s, const char *p)
{
    while (*p) { if (*s++ != *p++) return 0; }
    return 1;
}

static void cmd_help(void)
{
    puts_uart("commands:\n");
    puts_uart("  help      this message\n");
    puts_uart("  info      chip and boot info\n");
    puts_uart("  mem       memory layout\n");
    puts_uart("  regs      selected CSRs\n");
    puts_uart("  echo TEXT echo text back\n");
#ifndef V2
    puts_uart("  ota SIZE  receive OTA firmware over UART, write to ota_0, reboot\n");
#endif
}

static void cmd_info(uint32_t boot_cycles)
{
    uint32_t elapsed = sys_ticks() - boot_cycles;
    puts_uart("chip:  ESP32-C3 RV32IMC\n");
    puts_uart("hart:  "); put_dec(csr_mhartid()); puts_uart("\n");
    puts_uart("ticks: "); put_dec(elapsed); puts_uart(" SYSTIMER ticks since app start\n");
    puts_uart("uptime ~"); put_dec(elapsed / 16000u); puts_uart(" ms (16 MHz timer)\n");
}

static void cmd_mem(void)
{
    uint32_t ie = (uint32_t)(uintptr_t)&_image_end;
    uint32_t sp = current_sp();
    print_kv_hex("image_end ", ie);
    print_kv_hex("stack_ptr ", sp);
    puts_uart("free above image: "); put_dec(sp > ie ? sp - ie : 0); puts_uart(" bytes\n");
}

static void cmd_regs(void)
{
    print_kv_hex("mstatus ", csr_mstatus());
    print_kv_hex("mtvec   ", csr_mtvec());
    print_kv_hex("sp      ", current_sp());
    print_kv_hex("systimer", sys_ticks());
    print_kv_hex("uart_st ", UART_STATUS_REG);
}

static void run_cmd(char *line, uint32_t boot_cycles)
{
    if (!*line || streq(line, "\r") || streq(line, "\n")) return;
    if (streq(line, "help") || streq(line, "?")) cmd_help();
    else if (streq(line, "info"))                cmd_info(boot_cycles);
    else if (streq(line, "mem"))                 cmd_mem();
    else if (streq(line, "regs"))                cmd_regs();
    else if (startswith(line, "echo "))          { puts_uart(line + 5); puts_uart("\n"); }
#ifndef V2
    else if (startswith(line, "ota "))           cmd_ota(line + 4);
#endif
    else { puts_uart("unknown: "); puts_uart(line); puts_uart("\ntype 'help'\n"); }
}

/* ---- main ------------------------------------------------------------ */

void app_main(void)
{
    uint32_t boot_cycles = sys_ticks();

#ifdef V2
    mark_ota_valid();
    puts_uart("\n=== ESP32-C3 Rust Bootloader — [OTA v2] ===\n");
    puts_uart("OTA update successful, firmware marked valid.\n");
#else
    puts_uart("\n=== ESP32-C3 Rust Bootloader — test app ===\n");
#endif
    puts_uart("UART0 115200 8N1  |  type 'help'\n");
    puts_uart("esp32c3> ");

    char line[96];
    int  len = 0, saw_cr = 0;

    for (;;) {
        int c = getc_uart();
        if (c < 0) continue;

        if (c == '\n' && saw_cr) { saw_cr = 0; continue; }

        if (c == '\r' || c == '\n') {
            saw_cr = (c == '\r');
            puts_uart("\n");
            line[len] = '\0';
            run_cmd(line, boot_cycles);
            len = 0;
            puts_uart("esp32c3> ");
        } else if (c == 0x08 || c == 0x7f) {
            saw_cr = 0;
            if (len > 0) { len--; puts_uart("\b \b"); }
        } else if (c >= 0x20 && c <= 0x7e) {
            saw_cr = 0;
            if (len + 1 < (int)sizeof(line)) {
                line[len++] = (char)c;
                putc_uart((char)c);
            }
        }
    }
}
