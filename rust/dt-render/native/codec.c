#include <avif/avif.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
// Input RGB planes stay borrowed until this synchronous call returns. The output is owned by C.
int dt_render_encode(const uint8_t *const *frames, uint32_t count, uint32_t width, uint32_t height,
                     uint32_t quality, uint32_t speed, uint32_t threads, const uint32_t *durations,
                     uint32_t plays, uint8_t **output, size_t *length) {
    avifEncoder *encoder = avifEncoderCreate();
    if (!encoder) return AVIF_RESULT_OUT_OF_MEMORY;
    encoder->maxThreads = (int)threads; encoder->speed = (int)speed;
    encoder->quality = (int)quality; encoder->timescale = 1000;
    // Match the RGB PSNR acceptance metric used by encoder_match.py at equal size.
    avifEncoderSetCodecSpecificOption(encoder, "tune", "psnr");
    encoder->repetitionCount = plays == 0 ? AVIF_REPETITION_COUNT_INFINITE : (int)(plays - 1);
    avifResult result = AVIF_RESULT_OK;
    for (uint32_t i = 0; i < count && result == AVIF_RESULT_OK; ++i) {
        avifImage *image = avifImageCreate(width, height, 8, AVIF_PIXEL_FORMAT_YUV420);
        if (!image) { result = AVIF_RESULT_OUT_OF_MEMORY; break; }
        image->colorPrimaries = AVIF_COLOR_PRIMARIES_BT709;
        image->transferCharacteristics = AVIF_TRANSFER_CHARACTERISTICS_SRGB;
        image->matrixCoefficients = AVIF_MATRIX_COEFFICIENTS_BT601;
        avifRGBImage rgb; avifRGBImageSetDefaults(&rgb, image);
        rgb.format = AVIF_RGB_FORMAT_RGB; rgb.pixels = (uint8_t *)frames[i]; rgb.rowBytes = width * 3;
        result = avifImageRGBToYUV(image, &rgb);
        if (result == AVIF_RESULT_OK) result = avifEncoderAddImage(encoder, image, durations[i], count == 1 ? AVIF_ADD_IMAGE_FLAG_SINGLE : 0);
        avifImageDestroy(image);
    }
    avifRWData data = AVIF_DATA_EMPTY;
    if (result == AVIF_RESULT_OK) result = avifEncoderFinish(encoder, &data);
    if (result == AVIF_RESULT_OK) {
        *output = (uint8_t *)malloc(data.size);
        if (!*output) result = AVIF_RESULT_OUT_OF_MEMORY;
        else { memcpy(*output, data.data, data.size); *length = data.size; }
    }
    avifRWDataFree(&data); avifEncoderDestroy(encoder);
    return result;
}
void dt_render_free(uint8_t *output) { free(output); }
