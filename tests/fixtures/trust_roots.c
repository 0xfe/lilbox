// Exercise the native trust-settings APIs used by rustls-native-certs, without
// contacting a server or reading any private keys/passwords. Counts only.
#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <stdio.h>

int main(void) {
    const SecTrustSettingsDomain domains[] = {
        kSecTrustSettingsDomainUser,
        kSecTrustSettingsDomainAdmin,
        kSecTrustSettingsDomainSystem,
    };
    for (unsigned int d = 0; d < 3; ++d) {
        CFArrayRef certificates = NULL;
        OSStatus status = SecTrustSettingsCopyCertificates(domains[d], &certificates);
        if (status == errSecNoTrustSettings && domains[d] != kSecTrustSettingsDomainSystem) {
            printf("domain %u: 0 certificates\n", d);
            continue;
        }
        if (status != errSecSuccess || certificates == NULL) {
            fprintf(stderr, "cannot load trust domain %u: OSStatus %d\n", d, (int)status);
            return 1;
        }
        CFIndex count = CFArrayGetCount(certificates);
        if (domains[d] == kSecTrustSettingsDomainSystem && count == 0) {
            fprintf(stderr, "no system root certificates available\n");
            CFRelease(certificates);
            return 1;
        }
        for (CFIndex i = 0; i < count; ++i) {
            SecCertificateRef certificate = (SecCertificateRef)CFArrayGetValueAtIndex(certificates, i);
            CFArrayRef settings = NULL;
            status = SecTrustSettingsCopyTrustSettings(certificate, domains[d], &settings);
            if (status != errSecSuccess) {
                fprintf(stderr, "cannot load certificate trust settings: OSStatus %d\n", (int)status);
                CFRelease(certificates);
                return 1;
            }
            if (settings != NULL) CFRelease(settings);
        }
        printf("domain %u: %ld certificates\n", d, (long)count);
        CFRelease(certificates);
    }
    return 0;
}
