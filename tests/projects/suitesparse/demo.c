#include <stdio.h>
#include <SuiteSparse_config.h>

int main(void) {
    int version[3];
    SuiteSparse_version(version);
    printf("SuiteSparse %d.%d.%d\n", version[0], version[1], version[2]);
    return 0;
}