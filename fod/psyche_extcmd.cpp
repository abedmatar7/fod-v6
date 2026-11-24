// Vendor-namespace helper to call Xiaomi HIDL 1.0 IXiaomiFingerprint::extCmd(cmd, param)
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <dlfcn.h>
#include <string>

namespace {

struct RawSp {
    void* ptr = nullptr;
    ~RawSp() {}
};

struct RawReturn {
    alignas(8) uint8_t storage[128];
    RawReturn() { std::memset(storage, 0, sizeof(storage)); }
    ~RawReturn() {}
};

using GetServiceFn = RawSp (*)(const std::string& serviceName, bool getStub);
using ExtCmdFn = RawReturn (*)(void* self, uint32_t cmd, uint32_t param);

static const char* kLibPath =
    "/vendor/lib64/vendor.xiaomi.hardware.fingerprintextension@1.0.so";
static const char* kSymGetService =
    "_ZN6vendor6xiaomi8hardware20fingerprintextension4V1_018IXiaomiFingerprint10getServiceERKNSt3__112basic_stringIcNS5_11char_traitsIcEENS5_9allocatorIcEEEEb";
static const char* kSymExtCmd =
    "_ZN6vendor6xiaomi8hardware20fingerprintextension4V1_021BpHwXiaomiFingerprint6extCmdEjj";

} // namespace

int main(int argc, char** argv) {
    void* handle = dlopen(kLibPath, RTLD_NOW);
    if (!handle) {
        std::fprintf(stderr, "psyche_extcmd: dlopen failed: %s\n", dlerror());
        return 1;
    }

    auto get_service = reinterpret_cast<GetServiceFn>(dlsym(handle, kSymGetService));
    auto ext_cmd = reinterpret_cast<ExtCmdFn>(dlsym(handle, kSymExtCmd));
    if (!get_service || !ext_cmd) {
        std::fprintf(stderr, "psyche_extcmd: dlsym failed: %s\n", dlerror());
        return 2;
    }

    std::string name("default");
    RawSp svc = get_service(name, false);
    if (!svc.ptr) {
        std::fprintf(stderr, "psyche_extcmd: IXiaomiFingerprint/default not available\n");
        return 3;
    }

    if (argc == 2 && std::strcmp(argv[1], "--pipe") == 0) {
        char buf[64];
        while (std::fgets(buf, sizeof(buf), stdin)) {
            uint32_t cmd = 0, param = 0;
            if (std::sscanf(buf, "%u %u", &cmd, &param) == 2) {
                (void)ext_cmd(svc.ptr, cmd, param);
            }
        }
        return 0;
    }

    if (argc == 3) {
        uint32_t cmd = static_cast<uint32_t>(std::strtoul(argv[1], nullptr, 0));
        uint32_t param = static_cast<uint32_t>(std::strtoul(argv[2], nullptr, 0));
        (void)ext_cmd(svc.ptr, cmd, param);
        std::printf("psyche_extcmd: sent (%u, %u)\n", cmd, param);
        return 0;
    }

    std::fprintf(stderr, "Usage: %s <cmd> <param> | --pipe\n", argv[0]);
    return 4;
}
