/* Chỉ dùng cho hộp đen HTTP: không kết nối Telegram. */
#include <stddef.h>
#include <unistd.h>
int td_create_client_id(void) { static int id; return ++id; }
void td_send(int client_id, const char *request) { (void)client_id; (void)request; }
const char *td_receive(double timeout) { (void)timeout; usleep(10000); return NULL; }
const char *td_execute(const char *request) { (void)request; return NULL; }
