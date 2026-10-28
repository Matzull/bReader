/* Scans a binary PGM (P5) file, or a raw 8-bit luma file, and prints what it finds.
 *
 *   scan image.pgm
 *   scan image.raw WIDTH HEIGHT
 *   scan image.png          (needs a library built with the default "image" feature)
 *
 * Build (Linux, after `cargo build --release -p breader-ffi`):
 *   cc -O2 -Icrates/breader-ffi/include crates/breader-ffi/examples/c/scan.c \
 *      -Ltarget/release -lbreader -Wl,-rpath,$PWD/target/release -o scan
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "breader.h"

static unsigned char *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    unsigned char *buf = NULL;
    long n;
    if (!f) return NULL;
    if (fseek(f, 0, SEEK_END) != 0 || (n = ftell(f)) < 0 || fseek(f, 0, SEEK_SET) != 0) {
        fclose(f);
        return NULL;
    }
    buf = malloc((size_t)n + 1);
    if (buf && fread(buf, 1, (size_t)n, f) == (size_t)n) {
        *len = (size_t)n;
    } else {
        free(buf);
        buf = NULL;
    }
    fclose(f);
    return buf;
}

/* Parses "P5 width height maxval" and returns the offset of the pixels, or 0. */
static size_t parse_pgm(const unsigned char *buf, size_t len, size_t *w, size_t *h) {
    size_t pos = 2, vals[3], got = 0;
    if (len < 10 || buf[0] != 'P' || buf[1] != '5') return 0;
    while (got < 3 && pos < len) {
        if (buf[pos] == '#') {
            while (pos < len && buf[pos] != '\n') pos++;
        } else if (buf[pos] >= '0' && buf[pos] <= '9') {
            size_t v = 0;
            while (pos < len && buf[pos] >= '0' && buf[pos] <= '9') v = v * 10 + (buf[pos++] - '0');
            vals[got++] = v;
        } else {
            pos++;
        }
    }
    if (got < 3 || vals[2] != 255 || pos >= len) return 0;
    *w = vals[0];
    *h = vals[1];
    return pos + 1; /* one whitespace byte after maxval */
}

int main(int argc, char **argv) {
    size_t len = 0, width = 0, height = 0, offset = 0, i, n;
    unsigned char *buf;
    BreaderReader *reader;
    BreaderResults *results = NULL;
    BreaderStatus st;
    int is_png;

    if (argc != 2 && argc != 4) {
        fprintf(stderr, "usage: %s image.pgm | image.png | image.raw WIDTH HEIGHT\n", argv[0]);
        return 2;
    }
    printf("breader %s\n", breader_version());
    reader = breader_reader_new(NULL);
    if (!reader) return 1;

    is_png = strlen(argv[1]) > 4 && (strcmp(argv[1] + strlen(argv[1]) - 4, ".png") == 0 ||
                                     strcmp(argv[1] + strlen(argv[1]) - 4, ".jpg") == 0);
    if (is_png) {
        st = breader_scan_file(reader, argv[1], &results);
    } else {
        buf = read_file(argv[1], &len);
        if (!buf) {
            fprintf(stderr, "cannot read %s\n", argv[1]);
            breader_reader_free(reader);
            return 1;
        }
        if (argc == 4) {
            width = (size_t)atol(argv[2]);
            height = (size_t)atol(argv[3]);
        } else if ((offset = parse_pgm(buf, len, &width, &height)) == 0) {
            fprintf(stderr, "not a binary 8-bit PGM\n");
            free(buf);
            breader_reader_free(reader);
            return 1;
        }
        st = breader_scan(reader, buf + offset, len - offset, width, height, width,
                          BREADER_PIXEL_LUMA8, &results);
        free(buf); /* the buffer is only read during the call */
    }
    if (st != BREADER_OK) {
        fprintf(stderr, "scan failed: %s\n", breader_status_string(st));
        breader_reader_free(reader);
        return 1;
    }

    n = breader_results_count(results);
    printf("%zu symbol(s)\n", n);
    for (i = 0; i < n; i++) {
        size_t dlen = 0;
        const char *text = breader_result_text(results, i, NULL);
        const char *hri = breader_result_gs1_hri(results, i);
        BreaderPoint c[4];
        (void)breader_result_data(results, i, &dlen);
        printf("%s [%s] %zu bytes\n", breader_result_symbology_name(results, i),
               breader_result_aim_id(results, i), dlen);
        if (text) printf("  text: %s\n", text);
        if (hri) printf("  gs1: %s\n", hri);
        if (breader_result_corners(results, i, c) == BREADER_OK)
            printf("  corners: (%.1f,%.1f) (%.1f,%.1f) (%.1f,%.1f) (%.1f,%.1f)\n", c[0].x, c[0].y,
                   c[1].x, c[1].y, c[2].x, c[2].y, c[3].x, c[3].y);
    }
    breader_results_free(results);
    breader_reader_free(reader);
    return 0;
}
