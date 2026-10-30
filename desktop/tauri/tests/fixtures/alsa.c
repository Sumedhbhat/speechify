/* Synthetic ALSA capture devices. No hardware is opened. The timer behaves
 * like a microphone clock; each PCM advertises one exact sample format. */
#include <alsa/asoundlib.h>
#include <alsa/pcm_external.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/timerfd.h>
#include <time.h>
#include <unistd.h>

struct microphone {
    snd_pcm_ioplug_t io;
    struct timespec started;
    int running;
    int prepare_error;
    int start_error;
    int disconnect;
    int startup_delay;
    int setup_gate;
};

static int start(snd_pcm_ioplug_t *io) {
    struct microphone *mic = io->private_data;
    if (mic->start_error) return -EIO;
    clock_gettime(CLOCK_MONOTONIC, &mic->started);
    mic->running = 1;
    struct itimerspec timer = {{0, 5000000}, {0, 5000000}};
    return timerfd_settime(io->poll_fd, 0, &timer, NULL) == 0 ? 0 : -errno;
}
static int stop(snd_pcm_ioplug_t *io) {
    struct microphone *mic = io->private_data;
    mic->running = 0;
    struct itimerspec timer = {0};
    timerfd_settime(io->poll_fd, 0, &timer, NULL);
    return 0;
}
static int prepare(snd_pcm_ioplug_t *io) {
    struct microphone *mic = io->private_data;
    if (mic->setup_gate) {
        const char *gate = getenv("SPEECHIFY_TEST_ALSA_GATE");
        const char *release = getenv("SPEECHIFY_TEST_ALSA_RELEASE");
        if (!gate || !release) return -EINVAL;
        int marker = open(gate, O_WRONLY | O_CREAT | O_TRUNC, 0600);
        if (marker >= 0) close(marker);
        for (unsigned attempt = 0; attempt < 10000 && access(release, F_OK) != 0; attempt++) usleep(1000);
        if (access(release, F_OK) != 0) return -ETIMEDOUT;
    }
    return mic->prepare_error ? -EIO : 0;
}
static snd_pcm_sframes_t position(snd_pcm_ioplug_t *io) {
    struct microphone *mic = io->private_data;
    if (!mic->running) return 0;
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    int64_t elapsed = (now.tv_sec - mic->started.tv_sec) * 1000000000LL
        + now.tv_nsec - mic->started.tv_nsec;
    if (mic->startup_delay) { elapsed -= 750000000; if (elapsed < 0) elapsed = 0; }
    if (mic->disconnect && elapsed > 100000000) return -ENODEV;
    return (elapsed * io->rate / 1000000000LL) % io->buffer_size;
}
static snd_pcm_sframes_t transfer(snd_pcm_ioplug_t *io,
        const snd_pcm_channel_area_t *areas, snd_pcm_uframes_t offset,
        snd_pcm_uframes_t frames) {
    for (snd_pcm_uframes_t frame = 0; frame < frames; frame++) {
        unsigned char *sample = (unsigned char *)areas[0].addr
            + areas[0].first / 8 + (offset + frame) * areas[0].step / 8;
        if (io->format == SND_PCM_FORMAT_FLOAT_LE) {
            float value = 0.25f;
            memcpy(sample, &value, sizeof(value));
        } else if (io->format == SND_PCM_FORMAT_S16_LE) {
            int16_t value = 8192;
            memcpy(sample, &value, sizeof(value));
        } else if (io->format == SND_PCM_FORMAT_U16_LE) {
            uint16_t value = 40960;
            memcpy(sample, &value, sizeof(value));
        } else {
            *sample = 32;
        }
    }
    return frames;
}
static int revents(snd_pcm_ioplug_t *io, struct pollfd *fds,
        unsigned int count, unsigned short *events) {
    if (!count) return -EINVAL;
    *events = fds[0].revents;
    if (*events & POLLIN) {
        uint64_t ticks;
        ssize_t bytes = read(io->poll_fd, &ticks, sizeof(ticks));
        if (bytes < 0 && errno != EAGAIN) return -errno;
    }
    return 0;
}
static int close_microphone(snd_pcm_ioplug_t *io) {
    close(io->poll_fd);
    free(io->private_data);
    return 0;
}
static const snd_pcm_ioplug_callback_t callbacks = {
    .start = start, .stop = stop, .prepare = prepare, .pointer = position,
    .transfer = transfer, .poll_revents = revents, .close = close_microphone,
};

SND_PCM_PLUGIN_DEFINE_FUNC(speechify_fixture) {
    (void)root;
    if (stream != SND_PCM_STREAM_CAPTURE) return -EINVAL;
    struct microphone *mic = calloc(1, sizeof(*mic));
    if (!mic) return -ENOMEM;
    unsigned int format = SND_PCM_FORMAT_FLOAT_LE;
    snd_config_iterator_t node, next;
    snd_config_for_each(node, next, conf) {
        snd_config_t *item = snd_config_iterator_entry(node);
        const char *id;
        snd_config_get_id(item, &id);
        if (!strcmp(id, "type") || !strcmp(id, "hint") || !strcmp(id, "comment")) continue;
        const char *value;
        if (snd_config_get_string(item, &value) < 0) { free(mic); return -EINVAL; }
        if (!strcmp(id, "format")) {
            format = snd_pcm_format_value(value);
        } else if (!strcmp(id, "behavior")) {
            mic->prepare_error = !strcmp(value, "prepare-error");
            mic->start_error = !strcmp(value, "start-error");
            mic->disconnect = !strcmp(value, "disconnect");
            mic->startup_delay = !strcmp(value, "startup-delay");
            mic->setup_gate = !strcmp(value, "setup-gate");
        } else { free(mic); return -EINVAL; }
    }
    mic->io.version = SND_PCM_IOPLUG_VERSION;
    mic->io.name = "Speechify synthetic capture";
    mic->io.callback = &callbacks;
    mic->io.private_data = mic;
    mic->io.poll_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    mic->io.poll_events = POLLIN;
    if (mic->io.poll_fd < 0) { int error = -errno; free(mic); return error; }
    int error = snd_pcm_ioplug_create(&mic->io, name, stream, mode);
    if (error < 0) { close_microphone(&mic->io); return error; }
    unsigned int access = SND_PCM_ACCESS_RW_INTERLEAVED;
    unsigned int rate = 16000, channels = 1;
    if ((error = snd_pcm_ioplug_set_param_list(&mic->io, SND_PCM_IOPLUG_HW_ACCESS, 1, &access)) < 0
        || (error = snd_pcm_ioplug_set_param_list(&mic->io, SND_PCM_IOPLUG_HW_FORMAT, 1, &format)) < 0
        || (error = snd_pcm_ioplug_set_param_list(&mic->io, SND_PCM_IOPLUG_HW_RATE, 1, &rate)) < 0
        || (error = snd_pcm_ioplug_set_param_list(&mic->io, SND_PCM_IOPLUG_HW_CHANNELS, 1, &channels)) < 0
        || (error = snd_pcm_ioplug_set_param_minmax(&mic->io, SND_PCM_IOPLUG_HW_PERIOD_BYTES, 128, 4096)) < 0
        || (error = snd_pcm_ioplug_set_param_minmax(&mic->io, SND_PCM_IOPLUG_HW_BUFFER_BYTES, 256, 16384)) < 0) {
        snd_pcm_ioplug_delete(&mic->io);
        return error;
    }
    *pcmp = mic->io.pcm;
    return 0;
}
SND_PCM_PLUGIN_SYMBOL(speechify_fixture)
