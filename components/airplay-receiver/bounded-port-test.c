#include "common.h"
#include <assert.h>
#include <dirent.h>
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

int _safe_socket_close(const char *filename, const int line, int *fd) {
  (void)filename; (void)line;
  if (*fd >= 0) { int r=close(*fd); *fd=-1; return r; }
  return 0;
}
static int fd_count(void) { DIR *d=opendir("/proc/self/fd"); assert(d); int n=0; while(readdir(d)) n++; closedir(d); return n; }
typedef struct { int fd; uint16_t port; int error; } result_t;
static void *parallel_bind(void *arg) { result_t *r=arg; r->error=bind_socket_and_port_range(SOCK_STREAM,AF_INET,"127.0.0.1",0,24600,10,&r->port,&r->fd); return NULL; }
int main(void) {
  uint16_t p=99; int fd=99;
  assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET,"127.0.0.1",0,0,10,&p,&fd)==EINVAL && p==0 && fd==-1);
  assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET,"127.0.0.1",0,65530,10,&p,&fd)==EINVAL);
  assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET,"not-an-address",0,24500,1,&p,&fd)==EINVAL && fd==-1);
  int base_fds=fd_count();
  int tcp1=-1,tcp2=-1,udp=-1; uint16_t p1=0,p2=0,pu=0;
  assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET,"127.0.0.1",0,24500,2,&p1,&tcp1)==0 && p1==24500);
  assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET,"127.0.0.1",0,24500,2,&p2,&tcp2)==0 && p2==24501);
  assert(bind_socket_and_port_range(SOCK_DGRAM,AF_INET,"127.0.0.1",0,24500,2,&pu,&udp)==0 && pu==24500);
  int exhausted=-1; uint16_t pe=0; assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET,"127.0.0.1",0,24500,2,&pe,&exhausted)==EADDRINUSE && exhausted==-1 && pe==0);
  close(tcp1);close(tcp2);close(udp); assert(fd_count()==base_fds);
#ifdef AF_INET6
  int six=-1; uint16_t p6=0; assert(bind_socket_and_port_range(SOCK_STREAM,AF_INET6,"::1",0,24510,1,&p6,&six)==0 && p6==24510); close(six);
#endif
  result_t results[4]={{.fd=-1},{.fd=-1},{.fd=-1},{.fd=-1}}; pthread_t threads[4];
  for(int i=0;i<4;i++) assert(pthread_create(&threads[i],NULL,parallel_bind,&results[i])==0);
  for(int i=0;i<4;i++) { pthread_join(threads[i],NULL); assert(results[i].error==0); for(int j=0;j<i;j++) assert(results[i].port!=results[j].port); }
  for(int i=0;i<4;i++) close(results[i].fd);
  assert(fd_count()==base_fds); puts("PASS bounded port helper"); return 0;
}
