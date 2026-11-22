#include <errno.h>
#include <libproc.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>

static int fail(const char *message) {
    fprintf(stderr, "%s\n", message);
    return 1;
}

static int parse_pid(const char *text, pid_t *pid) {
    char *end = NULL;
    errno = 0;
    long value = strtol(text, &end, 10);
    if (errno != 0 || end == text || *end != '\0' || value <= 0 || value > INT32_MAX) {
        return -1;
    }
    *pid = (pid_t)value;
    return 0;
}

static int process_path(pid_t pid, char *path, size_t size) {
    int length = proc_pidpath(pid, path, (uint32_t)size);
    if (length <= 0 || (size_t)length >= size) {
        return -1;
    }
    path[length] = '\0';
    return 0;
}

static int matches_path(pid_t pid, const char *expected) {
    char actual[PROC_PIDPATHINFO_MAXSIZE];
    char canonical_actual[PROC_PIDPATHINFO_MAXSIZE];
    char canonical_expected[PROC_PIDPATHINFO_MAXSIZE];
    if (process_path(pid, actual, sizeof(actual)) != 0 ||
        realpath(actual, canonical_actual) == NULL ||
        realpath(expected, canonical_expected) == NULL) {
        return -1;
    }
    return strcmp(canonical_actual, canonical_expected) == 0 ? 0 : -1;
}

static int report_usage(pid_t pid, const char *expected) {
    if (expected != NULL && matches_path(pid, expected) != 0) {
        return fail("process executable does not match the requested app");
    }

    struct rusage_info_v4 usage = {0};
    if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&usage) != 0) {
        return fail("proc_pid_rusage failed");
    }

    printf("{\"pid\":%d,\"physical_footprint_bytes\":%llu,\"resident_bytes\":%llu,"
           "\"lifetime_peak_footprint_bytes\":%llu}\n",
           pid,
           (unsigned long long)usage.ri_phys_footprint,
           (unsigned long long)usage.ri_resident_size,
           (unsigned long long)usage.ri_lifetime_max_phys_footprint);
    return 0;
}

static int terminate_process(pid_t pid, const char *expected, int signal_number) {
    if (matches_path(pid, expected) != 0) {
        return fail("refusing to signal a process that is not the measured app");
    }
    if (kill(pid, signal_number) != 0) {
        return fail("could not stop the measured app");
    }
    puts("{\"terminated\":true}");
    return 0;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--self") == 0) {
        return report_usage(getpid(), NULL);
    }

    if (argc == 4 && strcmp(argv[1], "--pid") == 0) {
        pid_t pid;
        if (parse_pid(argv[2], &pid) != 0) {
            return fail("PID must be a positive integer");
        }
        return report_usage(pid, argv[3]);
    }

    if (argc == 5 && strcmp(argv[1], "--terminate") == 0) {
        pid_t pid;
        if (parse_pid(argv[2], &pid) != 0) {
            return fail("PID must be a positive integer");
        }
        int signal_number = strcmp(argv[4], "term") == 0 ? SIGTERM :
                            strcmp(argv[4], "kill") == 0 ? SIGKILL : 0;
        if (signal_number == 0) {
            return fail("signal must be term or kill");
        }
        return terminate_process(pid, argv[3], signal_number);
    }

    return fail("usage: usage --self | --pid PID EXECUTABLE | --terminate PID EXECUTABLE term|kill");
}
