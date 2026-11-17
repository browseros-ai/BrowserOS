// OpenH264's C++ objects and versioned structs stay on this side of the ABI.
#include <wels/codec_api.h>
#include <climits>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <new>

struct sv_oh_enc {
    ISVCEncoder *codec = nullptr;
    SEncParamExt params = {};
    unsigned char *buffer = nullptr;
    size_t capacity = 0;
};

extern "C" void sv_oh_close(sv_oh_enc *e) {
    if (!e) return;
    if (e->codec) {
        e->codec->Uninitialize();
        WelsDestroySVCEncoder(e->codec);
    }
    std::free(e->buffer);
    delete e;
}

static int quantizer(float crf) {
    return static_cast<int>(std::round(crf));
}

extern "C" sv_oh_enc *sv_oh_open(int w, int h, int threads, float crf, int kbps,
                                const char *) {
    sv_oh_enc *e = new (std::nothrow) sv_oh_enc;
    if (!e) return nullptr;
    if (WelsCreateSVCEncoder(&e->codec) != 0 || !e->codec) {
        sv_oh_close(e);
        return nullptr;
    }
    int trace = WELS_LOG_ERROR;
    e->codec->SetOption(ENCODER_OPTION_TRACE_LEVEL, &trace);
    if (e->codec->GetDefaultParams(&e->params) != 0) {
        sv_oh_close(e);
        return nullptr;
    }
    SEncParamExt &p = e->params;
    p.iUsageType = SCREEN_CONTENT_REAL_TIME;
    p.iPicWidth = w;
    p.iPicHeight = h;
    p.fMaxFrameRate = 30;
    p.iTemporalLayerNum = 1;
    p.iSpatialLayerNum = 1;
    p.iComplexityMode = LOW_COMPLEXITY;
    p.uiIntraPeriod = 0;
    p.iNumRefFrame = 1;
    p.bEnableLongTermReference = false;
    p.bEnableFrameSkip = false;
    p.bEnableDenoise = false;
    p.bEnableBackgroundDetection = false;
    p.bEnableAdaptiveQuant = false;
    p.bEnableSceneChangeDetect = false;
    p.bPrefixNalAddingCtrl = false;
    p.bEnableSSEI = false;
    p.eSpsPpsIdStrategy = CONSTANT_ID;
    p.iMultipleThreadIdc = static_cast<unsigned short>(threads);
    p.iRCMode = kbps > 0 ? RC_BITRATE_MODE : RC_OFF_MODE;
    p.iTargetBitrate = kbps > 0 ? kbps * 1000 : 8000000;
    p.iMaxBitrate = UNSPECIFIED_BIT_RATE;
    p.iMinQp = quantizer(crf);
    p.iMaxQp = 51;
    SSpatialLayerConfig &layer = p.sSpatialLayers[0];
    layer.iVideoWidth = w;
    layer.iVideoHeight = h;
    layer.fFrameRate = 30;
    layer.iSpatialBitrate = p.iTargetBitrate;
    layer.iMaxSpatialBitrate = UNSPECIFIED_BIT_RATE;
    layer.uiProfileIdc = PRO_BASELINE;
    layer.iDLayerQp = quantizer(crf);
    layer.sSliceArgument.uiSliceMode = SM_FIXEDSLCNUM_SLICE;
    layer.sSliceArgument.uiSliceNum = threads;
    layer.bVideoSignalTypePresent = true;
    layer.uiVideoFormat = 5;
    layer.bFullRange = false;
    layer.bColorDescriptionPresent = true;
    layer.uiColorPrimaries = 1;
    layer.uiTransferCharacteristics = 1;
    layer.uiColorMatrix = 1;
    if (e->codec->InitializeExt(&p) != 0) {
        sv_oh_close(e);
        return nullptr;
    }
    return e;
}

static int apply(sv_oh_enc *e, SEncParamExt &p) {
    if (e->codec->SetOption(ENCODER_OPTION_SVC_ENCODE_PARAM_EXT, &p) != 0) return -1;
    // ParamAdjust copies iRCMode, but only this option refreshes the RC function pointers.
    int mode = p.iRCMode;
    return e->codec->SetOption(ENCODER_OPTION_RC_MODE, &mode);
}

extern "C" int sv_oh_reconfig(sv_oh_enc *e, float crf, int kbps) {
    SEncParamExt p = e->params;
    p.sSpatialLayers[0].iDLayerQp = quantizer(crf);
    if (p.iRCMode != RC_OFF_MODE && kbps > 0) {
        SBitrateInfo rate = {};
        rate.iLayer = SPATIAL_LAYER_ALL;
        rate.iBitrate = kbps * 1000;
        if (e->codec->SetOption(ENCODER_OPTION_BITRATE, &rate) != 0) return -1;
        p.iTargetBitrate = kbps * 1000;
        p.sSpatialLayers[0].iSpatialBitrate = p.iTargetBitrate;
    } else if (apply(e, p) != 0) {
        return -1;
    }
    e->params = p;
    return 0;
}

extern "C" int sv_oh_encode(sv_oh_enc *e, const unsigned char *y,
                             const unsigned char *u, const unsigned char *v,
                             int stride_y, int stride_uv, int idr, int qp,
                             long long pts_ms, unsigned char **out, int *key,
                             long long *out_pts) {
    *out = nullptr;
    *key = 0;
    *out_pts = -1;
    if (qp >= 0) {
        SEncParamExt sharp = e->params;
        sharp.iRCMode = RC_OFF_MODE;
        sharp.iMinQp = qp;
        sharp.sSpatialLayers[0].iDLayerQp = qp;
        if (apply(e, sharp) != 0) return -1;
    }
    // OpenH264 has no x264-style residual refinement. Recode the entire still as an IDR.
    int rc = (idr || qp >= 0) ? e->codec->ForceIntraFrame(true) : 0;
    SSourcePicture pic = {};
    pic.iColorFormat = videoFormatI420;
    pic.iPicWidth = e->params.iPicWidth;
    pic.iPicHeight = e->params.iPicHeight;
    pic.iStride[0] = stride_y;
    pic.iStride[1] = pic.iStride[2] = stride_uv;
    pic.pData[0] = const_cast<unsigned char *>(y);
    pic.pData[1] = const_cast<unsigned char *>(u);
    pic.pData[2] = const_cast<unsigned char *>(v);
    pic.uiTimeStamp = pts_ms;
    SFrameBSInfo info = {};
    if (rc == 0) rc = e->codec->EncodeFrame(&pic, &info);
    if (qp >= 0 && apply(e, e->params) != 0) return -1;
    if (rc != 0 || info.eFrameType == videoFrameTypeSkip) return -1;
    size_t size = 0;
    for (int i = 0; i < info.iLayerNum; ++i) {
        const SLayerBSInfo &layer = info.sLayerInfo[i];
        for (int j = 0; j < layer.iNalCount; ++j) {
            if (layer.pNalLengthInByte[j] <= 0) return -1;
            size += static_cast<size_t>(layer.pNalLengthInByte[j]);
        }
    }
    if (size == 0 || size > INT_MAX) return -1;
    if (size > e->capacity) {
        void *buffer = std::realloc(e->buffer, size);
        if (!buffer) return -1;
        e->buffer = static_cast<unsigned char *>(buffer);
        e->capacity = size;
    }
    size_t offset = 0;
    for (int i = 0; i < info.iLayerNum; ++i) {
        const SLayerBSInfo &layer = info.sLayerInfo[i];
        size_t bytes = 0;
        for (int j = 0; j < layer.iNalCount; ++j) bytes += layer.pNalLengthInByte[j];
        std::memcpy(e->buffer + offset, layer.pBsBuf, bytes);
        offset += bytes;
    }
    *out = e->buffer;
    *key = info.eFrameType == videoFrameTypeIDR;
    *out_pts = info.uiTimeStamp;
    return static_cast<int>(size);
}
