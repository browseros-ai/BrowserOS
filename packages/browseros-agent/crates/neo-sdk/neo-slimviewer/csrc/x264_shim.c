// The whole surface slimviewer needs from libx264, as five plain C calls, so the Rust side
// never has to mirror x264_param_t (a large struct whose layout changes between x264 builds).
//
// Settings: no lookahead and no B-frames (tune zerolatency: every frame comes out of the call
// that took it in), no periodic keyframes, Annex B start codes with SPS/PPS before each IDR (what
// WebCodecs on iOS decodes), CRF quality that the caller moves live. VBV only when asked
// (vbv_kbps > 0): the caller uses it for half-size motion on a short link. A full-size encoder has
// none, because a VBV buffer starves the keyframe of a still screen (QP 33 instead of 21).
#include <stdint.h>
#include <stdlib.h>
#include <x264.h>

typedef struct sv_enc {
    x264_t *x;
    x264_param_t p;
    x264_picture_t pic;
} sv_enc;

static void set_vbv(x264_param_t *p, int vbv_kbps) {
    p->rc.i_vbv_max_bitrate = vbv_kbps;
    // 150 ms of buffer: one frame may not take much more than a frame interval or two on the link.
    p->rc.i_vbv_buffer_size = vbv_kbps * 3 / 20;
}

sv_enc *sv_open(int w, int h, int threads, float crf, int vbv_kbps, const char *preset) {
    sv_enc *e = calloc(1, sizeof *e);
    if (!e) return NULL;
    if (x264_param_default_preset(&e->p, preset, "zerolatency") < 0) goto fail;
    e->p.i_width = w;
    e->p.i_height = h;
    e->p.i_csp = X264_CSP_I420;
    e->p.i_threads = threads;
    e->p.b_sliced_threads = threads > 1;
    // Constant-rate timing on purpose: in VFR mode x264 holds every picture until the next one
    // arrives (it needs the next timestamp to size the current frame), one frame of lag.
    e->p.b_vfr_input = 0;
    e->p.i_timebase_num = 1;
    e->p.i_timebase_den = 1000;
    e->p.i_fps_num = 30;
    e->p.i_fps_den = 1;
    e->p.i_keyint_max = X264_KEYINT_MAX_INFINITE;
    e->p.i_scenecut_threshold = 0;
    e->p.b_repeat_headers = 1;
    e->p.b_annexb = 1;
    e->p.i_log_level = X264_LOG_NONE;
    e->p.vui.i_colorprim = 1; // BT.709, matching yuv.rs
    e->p.vui.i_transfer = 1;
    e->p.vui.i_colmatrix = 1;
    e->p.vui.b_fullrange = 0;
    // ultrafast codes intra blocks as 16x16 only; text needs 4x4 (a keyframe of a text page: 29 dB
    // → 46 dB at the same CRF). Inter analysis stays ultrafast.
    e->p.analyse.intra = X264_ANALYSE_I4x4;
    // ultrafast's diamond search (±16 px) loses a scroll: at 2x, 1000 px/s moves ~66 px between
    // frames, and every such frame was coded almost from scratch (~120 KB). The uneven
    // multi-hexagon search finds it (~7 KB, and faster: less residual to code).
    e->p.analyse.i_me_method = X264_ME_UMH;
    e->p.analyse.i_me_range = 128;
    e->p.rc.i_rc_method = X264_RC_CRF;
    e->p.rc.f_rf_constant = crf;
    if (vbv_kbps > 0) set_vbv(&e->p, vbv_kbps);
    e->x = x264_encoder_open(&e->p);
    if (!e->x) goto fail;
    x264_picture_init(&e->pic);
    e->pic.img.i_csp = X264_CSP_I420;
    e->pic.img.i_plane = 3;
    return e;
fail:
    free(e);
    return NULL;
}

// Encodes one I420 picture. qp >= 0 forces that quantizer for this frame (the still-screen
// sharpen); idr forces a keyframe. Returns the access unit's size and points *out at it (valid
// until the next call), or -1.
int sv_encode(sv_enc *e, const uint8_t *y, const uint8_t *u, const uint8_t *v, int stride_y,
              int stride_uv, int idr, int qp, int64_t pts_ms, uint8_t **out, int *key,
              int64_t *out_pts) {
    x264_nal_t *nal = NULL;
    int nnal = 0;
    x264_picture_t done;
    e->pic.img.plane[0] = (uint8_t *)y;
    e->pic.img.plane[1] = (uint8_t *)u;
    e->pic.img.plane[2] = (uint8_t *)v;
    e->pic.img.i_stride[0] = stride_y;
    e->pic.img.i_stride[1] = stride_uv;
    e->pic.img.i_stride[2] = stride_uv;
    e->pic.i_type = idr ? X264_TYPE_IDR : X264_TYPE_AUTO;
    e->pic.i_qpplus1 = qp >= 0 ? qp + 1 : X264_QP_AUTO;
    e->pic.i_pts = pts_ms;
    // A forced quantizer is the still-screen sharpen: a P-frame of an unchanged picture. With
    // x264's coefficient decimation and fast skip it codes nothing (all its residual is small
    // coefficients), so both are off for that one frame.
    if (qp >= 0) {
        e->p.analyse.b_dct_decimate = 0;
        e->p.analyse.b_fast_pskip = 0;
        x264_encoder_reconfig(e->x, &e->p);
    }
    int size = x264_encoder_encode(e->x, &nal, &nnal, &e->pic, &done);
    if (qp >= 0) {
        e->p.analyse.b_dct_decimate = 1;
        e->p.analyse.b_fast_pskip = 1;
        x264_encoder_reconfig(e->x, &e->p);
    }
    if (size < 0) return -1;
    // x264 lays out one frame's NAL units back to back, starting at the first payload.
    *out = size > 0 ? nal[0].p_payload : NULL;
    *key = size > 0 ? done.b_keyframe : 0;
    *out_pts = size > 0 ? done.i_pts : -1;
    return size;
}

// vbv_kbps is ignored unless the encoder was opened with VBV.
int sv_reconfig(sv_enc *e, float crf, int vbv_kbps) {
    e->p.rc.f_rf_constant = crf;
    if (e->p.rc.i_vbv_max_bitrate > 0 && vbv_kbps > 0) set_vbv(&e->p, vbv_kbps);
    return x264_encoder_reconfig(e->x, &e->p);
}

void sv_close(sv_enc *e) {
    if (!e) return;
    x264_encoder_close(e->x);
    free(e);
}
