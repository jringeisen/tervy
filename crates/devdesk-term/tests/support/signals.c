// Direct exec probe: a shell or interpreter could reset signals before we see
// them. This reports the dispositions and mask inherited at program entry.
#include <signal.h>
#include <stdio.h>
int main(void) {
    const int signals[] = {SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGALRM, SIGPIPE, SIGCHLD};
    sigset_t blocked;
    if (sigprocmask(0, NULL, &blocked)) return 2;
    for (unsigned i = 0; i < sizeof(signals)/sizeof(signals[0]); ++i) {
        struct sigaction action;
        if (sigaction(signals[i], NULL, &action)) return 3;
        printf("SIGNAL %d default=%d blocked=%d\n", signals[i],
               action.sa_handler == SIG_DFL, sigismember(&blocked, signals[i]));
    }
    return 0;
}
