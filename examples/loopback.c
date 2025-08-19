#include <endian.h>
#include <infiniband/verbs.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <fcntl.h>

#define BUF_SIZE (32UL * 1024 * 1024)
#define MSG_LEN (0x1000 - 1023)

const uint64_t SRC_BUFFER_OFFSET = 0;
const uint64_t DST_BUFFER_OFFSET = BUF_SIZE;

void die(const char *reason);
void printZeroRanges(char *dst_buffer, int msg_len);

void die(const char *reason) {
  perror(reason);
  exit(EXIT_FAILURE);
}

int run_single_mr(int msg_len) {
  struct ibv_device **dev_list;
  struct ibv_context *context;
  struct ibv_pd *pd;
  struct ibv_mr *mr;
  struct ibv_qp *qp0;
  struct ibv_qp *qp1;
  struct ibv_qp_init_attr qp_init_attr = {0};
  struct ibv_cq *send_cq;
  struct ibv_cq *recv_cq;
  char *buffer;
  char *src_buffer;
  char *dst_buffer;
  int num_devices;

  buffer = mmap(NULL, BUF_SIZE * 2, PROT_READ | PROT_WRITE,
		MAP_SHARED | MAP_ANONYMOUS | MAP_HUGETLB | MAP_POPULATE, -1, 0);
  if (buffer == MAP_FAILED) {
    die("Map failed");
  }

#ifdef COMPILE_FOR_RTL_SIMULATOR_TEST
  buffer = (char*)0x7f7e8e600000;
#endif

  src_buffer = buffer;
  dst_buffer = buffer + BUF_SIZE;
  dev_list = ibv_get_device_list(&num_devices);
  if (!dev_list) {
    die("Failed to get device list");
  }
  context = ibv_open_device(dev_list[0]);
  pd = ibv_alloc_pd(context);
  send_cq = ibv_create_cq(context, 512, NULL, NULL, 0);
  recv_cq = ibv_create_cq(context, 512, NULL, NULL, 0);

  if (!send_cq || !recv_cq) {
    die("Error creating CQ");
  }

  qp_init_attr.qp_type = IBV_QPT_RC;
  qp_init_attr.cap.max_send_wr = 100;
  qp_init_attr.cap.max_recv_wr = 100;
  qp_init_attr.cap.max_send_sge = 100;
  qp_init_attr.cap.max_recv_sge = 100;
  qp_init_attr.send_cq = send_cq;
  qp_init_attr.recv_cq = recv_cq;

  qp0 = ibv_create_qp(pd, &qp_init_attr);
  qp1 = ibv_create_qp(pd, &qp_init_attr);
  struct ibv_qp_attr qp_attr = {.qp_state = IBV_QPS_INIT,
				.pkey_index = 0,
				.port_num = 1,
				.qp_access_flags = IBV_ACCESS_LOCAL_WRITE |
						   IBV_ACCESS_REMOTE_READ |
						   IBV_ACCESS_REMOTE_WRITE};
  if (ibv_modify_qp(qp0, &qp_attr,
		    IBV_QP_STATE | IBV_QP_PKEY_INDEX | IBV_QP_PORT |
			IBV_QP_ACCESS_FLAGS)) {
    die("Failed to modify QP0 to INIT");
  }
  if (ibv_modify_qp(qp1, &qp_attr,
		    IBV_QP_STATE | IBV_QP_PKEY_INDEX | IBV_QP_PORT |
			IBV_QP_ACCESS_FLAGS)) {
    die("Failed to modify QP1 to INIT");
  }

  qp_attr.qp_state = IBV_QPS_RTS;
  qp_attr.path_mtu = IBV_MTU_4096;
  qp_attr.dest_qp_num = qp1->qp_num;
  qp_attr.rq_psn = 0;
  qp_attr.ah_attr.port_num = 1;
  uint32_t ipv4_addr = 0x1122330A;
  qp_attr.ah_attr.grh.dgid.raw[10] = 0xFF;
  qp_attr.ah_attr.grh.dgid.raw[11] = 0xFF;
  qp_attr.ah_attr.grh.dgid.raw[12] = (ipv4_addr >> 24) & 0xFF;
  qp_attr.ah_attr.grh.dgid.raw[13] = (ipv4_addr >> 16) & 0xFF;
  qp_attr.ah_attr.grh.dgid.raw[14] = (ipv4_addr >> 8) & 0xFF;
  qp_attr.ah_attr.grh.dgid.raw[15] = ipv4_addr & 0xFF;

  if (ibv_modify_qp(qp0, &qp_attr,
		    IBV_QP_STATE | IBV_QP_AV | IBV_QP_PATH_MTU |
			IBV_QP_DEST_QPN | IBV_QP_RQ_PSN |
			IBV_QP_MAX_DEST_RD_ATOMIC | IBV_QP_MIN_RNR_TIMER)) {
    fprintf(stderr, "Failed to modify QP0 to RTR\n");
    return 1;
  }
  qp_attr.dest_qp_num = qp0->qp_num;
  if (ibv_modify_qp(qp1, &qp_attr,
		    IBV_QP_STATE | IBV_QP_AV | IBV_QP_PATH_MTU |
			IBV_QP_DEST_QPN | IBV_QP_RQ_PSN |
			IBV_QP_MAX_DEST_RD_ATOMIC | IBV_QP_MIN_RNR_TIMER)) {
    fprintf(stderr, "Failed to modify QP1 to RTR\n");
    return 1;
  }

  memset(src_buffer, 'a', msg_len);
  memset(dst_buffer, 0, msg_len);

  mr = ibv_reg_mr(pd, buffer, BUF_SIZE * 2,
		  IBV_ACCESS_LOCAL_WRITE | IBV_ACCESS_REMOTE_WRITE |
		      IBV_ACCESS_REMOTE_READ);
  struct ibv_sge sge = {
      .addr = (uint64_t)src_buffer, .length = msg_len, .lkey = mr->lkey};
  struct ibv_send_wr wr = {.sg_list = &sge,
			   .num_sge = 1,
			   .opcode = IBV_WR_RDMA_WRITE,
			   .send_flags = IBV_SEND_SIGNALED,
			   .wr_id = 17,
			   .wr.rdma.remote_addr = (uint64_t)dst_buffer,
			   .wr.rdma.rkey = mr->lkey};
  struct ibv_send_wr *bad_wr;
  ibv_post_send(qp0, &wr, &bad_wr);
  struct ibv_wc wc = {0};

  while (ibv_poll_cq(send_cq, 1, &wc) == 0) {
    usleep(1000);
  }

  int cnt_valid = 0;
  for (int i = 0; i < msg_len; i++) {
    if (dst_buffer[i] == 'a') {
      cnt_valid += 1;
    }
  }
  printf("wc wr_id: %lu\n", wc.wr_id);
  printf("received bytes count: %d\n", cnt_valid);

  ibv_destroy_qp(qp0);
  ibv_dereg_mr(mr);
  ibv_dealloc_pd(pd);
  ibv_close_device(context);
  ibv_free_device_list(dev_list);

  if (cnt_valid != msg_len) {
    die("Failed to read the entire message");
  }

  return 0;
}

void printZeroRanges(char *dst_buffer, int msg_len) {
  int start = -1;

  for (int i = 0; i < msg_len; i++) {
    if (dst_buffer[i] == 0) {
      if (start == -1)
	start = i;
    } else {
      if (start != -1) {
	int length = i - start;
	printf("Zero range: %d-%d (length: %d)\n", start / 4096, i / 4096,
	       length);
	start = -1;
      }
    }
  }

  if (start != -1) {
    int length = msg_len - start;
    printf("Zero range: %d-%d (length: %d)\n", start / 4096, msg_len / 4096,
	   length);
  }
}

int main(int argc, char *argv[]) {
  if (argc < 2) {
    printf("Usage: %s <msg_len>\n", argv[0]);
    return 1;
  }
  int msg_len = atoi(argv[1]);
  run_single_mr(msg_len);
}
