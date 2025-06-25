#include <arpa/inet.h>
#include <infiniband/verbs.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <unistd.h>

#define BUF_SIZE (1024UL * 1024 * 1024)
#define PORT 12346

struct rdma_context {
  struct ibv_context *ctx;
  struct ibv_pd *pd;
  struct ibv_mr *mr;
  struct ibv_cq *cq;
  struct ibv_qp *qp;
  char *buffer;
};

void die(const char *reason);
void setup_ib(struct rdma_context *ctx, bool is_client);
void exchange_info(int sock, struct rdma_context *ctx, uint32_t *rkey,
		   uint64_t *raddr, uint32_t *dqpn);
void run_server(int msg_len);
void run_client(int msg_len, char *server_ip);
void setup_qp(struct rdma_context *ctx, uint32_t dqpn);

void die(const char *reason) {
  perror(reason);
  exit(EXIT_FAILURE);
}

void setup_ib(struct rdma_context *ctx, bool is_client) {
  struct ibv_device **dev_list = ibv_get_device_list(NULL);
  if (!dev_list)
    die("Failed to get IB devices list");

  ctx->ctx = ibv_open_device(dev_list[0]);
  if (!ctx->ctx)
    die("Failed to open IB device");

  ctx->pd = ibv_alloc_pd(ctx->ctx);
  if (!ctx->pd)
    die("Failed to allocate PD");

  ctx->buffer = mmap(NULL, BUF_SIZE, PROT_READ | PROT_WRITE,
		     MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB, -1, 0);
  ctx->mr = ibv_reg_mr(ctx->pd, ctx->buffer, BUF_SIZE,
		       IBV_ACCESS_LOCAL_WRITE | IBV_ACCESS_REMOTE_WRITE |
			   IBV_ACCESS_REMOTE_READ);
  if (!ctx->mr)
    die("Failed to register MR");

  ctx->cq = ibv_create_cq(ctx->ctx, 1, NULL, NULL, 0);
  if (!ctx->cq)
    die("Failed to create CQ");

  struct ibv_qp_init_attr qp_attr = {.send_cq = ctx->cq,
				     .recv_cq = ctx->cq,
				     .cap = {.max_send_wr = 1,
					     .max_recv_wr = 1,
					     .max_send_sge = 1,
					     .max_recv_sge = 1},
				     .qp_type = IBV_QPT_RC};
  ctx->qp = ibv_create_qp(ctx->pd, &qp_attr);
  if (!ctx->qp)
    die("Failed to create QP");

  ibv_free_device_list(dev_list);
}

void setup_qp(struct rdma_context *ctx, uint32_t dqpn) {
  struct ibv_qp_attr attr = {.qp_state = IBV_QPS_INIT,
			     .pkey_index = 0,
			     .port_num = 1,
			     .qp_access_flags = IBV_ACCESS_LOCAL_WRITE |
						IBV_ACCESS_REMOTE_WRITE |
						IBV_ACCESS_REMOTE_READ};

  if (ibv_modify_qp(ctx->qp, &attr,
		    IBV_QP_STATE | IBV_QP_PKEY_INDEX | IBV_QP_PORT |
			IBV_QP_ACCESS_FLAGS))
    die("Failed to transition QP to INIT");

  memset(&attr, 0, sizeof(attr));
  attr.qp_state = IBV_QPS_RTR;
  attr.path_mtu = IBV_MTU_4096;
  attr.dest_qp_num = dqpn;
  attr.rq_psn = 0;
  attr.max_dest_rd_atomic = 1;
  attr.min_rnr_timer = 12;
  attr.ah_attr.is_global = 0;
  attr.ah_attr.dlid = 0;
  attr.ah_attr.sl = 0;
  attr.ah_attr.src_path_bits = 0;
  attr.ah_attr.port_num = 1;

  if (ibv_modify_qp(ctx->qp, &attr,
		    IBV_QP_STATE | IBV_QP_AV | IBV_QP_PATH_MTU |
			IBV_QP_DEST_QPN | IBV_QP_RQ_PSN |
			IBV_QP_MAX_DEST_RD_ATOMIC | IBV_QP_MIN_RNR_TIMER))
    die("Failed to transition QP to RTR");

  memset(&attr, 0, sizeof(attr));
  attr.qp_state = IBV_QPS_RTS;
  attr.timeout = 14;
  attr.retry_cnt = 7;
  attr.rnr_retry = 7;
  attr.sq_psn = 0;
  attr.max_rd_atomic = 1;

  uint32_t ipv4_addr = 0x1122330A;
  attr.ah_attr.grh.dgid.raw[10] = 0xFF;
  attr.ah_attr.grh.dgid.raw[11] = 0xFF;
  attr.ah_attr.grh.dgid.raw[12] = (ipv4_addr >> 24) & 0xFF;
  attr.ah_attr.grh.dgid.raw[13] = (ipv4_addr >> 16) & 0xFF;
  attr.ah_attr.grh.dgid.raw[14] = (ipv4_addr >> 8) & 0xFF;
  attr.ah_attr.grh.dgid.raw[15] = ipv4_addr & 0xFF;

  if (ibv_modify_qp(ctx->qp, &attr,
		    IBV_QP_STATE | IBV_QP_AV | IBV_QP_TIMEOUT |
			IBV_QP_RETRY_CNT | IBV_QP_RNR_RETRY | IBV_QP_SQ_PSN |
			IBV_QP_MAX_QP_RD_ATOMIC))
    die("Failed to transition QP to RTS");
}

void exchange_info(int sock, struct rdma_context *ctx, uint32_t *rkey,
		   uint64_t *raddr, uint32_t *dqpn) {
  uint32_t lkey = ctx->mr->rkey;
  uint64_t addr = (uint64_t)ctx->buffer;
  uint32_t qpn = ctx->qp->qp_num;

  if (send(sock, &lkey, sizeof(lkey), 0) < 0 ||
      (send(sock, &addr, sizeof(addr), 0) < 0 ||
       send(sock, &qpn, sizeof(qpn), 0) < 0))
    die("Failed to send MR info");

  if (recv(sock, rkey, sizeof(*rkey), 0) < 0 ||
      (recv(sock, raddr, sizeof(*raddr), 0) < 0 ||
       recv(sock, dqpn, sizeof(*dqpn), 0) < 0))
    die("Failed to receive MR info");
}

void handshake(int sock) {
  char dummy = 0;

  if (send(sock, &dummy, sizeof(dummy), 0) < 0)
    die("Failed to send handshake");

  if (recv(sock, &dummy, sizeof(dummy), 0) < 0)
    die("Failed to receive handshake");
}

void run_server(int msg_len) {
  struct rdma_context ctx;
  setup_ib(&ctx, false);

  int sock = socket(AF_INET, SOCK_STREAM, 0);
  int opt = 1;
  if (setsockopt(sock, SOL_SOCKET, SO_REUSEPORT, &opt, sizeof(opt)) == -1) {
    die("setsockopt");
  }
  struct sockaddr_in addr = {.sin_family = AF_INET,
			     .sin_addr.s_addr = INADDR_ANY,
			     .sin_port = htons(PORT)};
  if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
    die("failed to bind to addr");
  }
  listen(sock, 1);

  printf("Server waiting for connection...\n");
  int client_sock = accept(sock, NULL, NULL);
  close(sock);

  memset(ctx.buffer, 0, BUF_SIZE);

  uint32_t rkey;
  uint64_t raddr;
  uint32_t dqpn;
  exchange_info(client_sock, &ctx, &rkey, &raddr, &dqpn);
  setup_qp(&ctx, dqpn);

  struct ibv_recv_wr wr = {0};
  struct ibv_recv_wr *bad_wr;
  struct ibv_sge sge = {
      .addr = (uint64_t)ctx.buffer, .length = BUF_SIZE, .lkey = ctx.mr->lkey};
  wr.sg_list = &sge;
  wr.num_sge = 1;

  ibv_post_recv(ctx.qp, &wr, &bad_wr);
  handshake(client_sock);

  long long cnt_valid = 0;
  struct ibv_wc wc = {0};

  while (ibv_poll_cq(ctx.cq, 1, &wc) < 1) {
    usleep(1000);
  }

  for (int i = 0; i < BUF_SIZE; i++) {
    if (ctx.buffer[i] == 'c') {
      cnt_valid++;
    }
  }

  printf("received bytes count: %lld\n", cnt_valid);

  close(client_sock);
}

void run_client(int msg_len, char *server_ip) {
  struct rdma_context ctx;

  setup_ib(&ctx, true);
  int sock = socket(AF_INET, SOCK_STREAM, 0);
  struct sockaddr_in addr = {.sin_family = AF_INET, .sin_port = htons(PORT)};
  inet_pton(AF_INET, server_ip, &addr.sin_addr);

  printf("connect");
  if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
    die("failed to connect");
  }
  uint32_t rkey;
  uint64_t raddr;
  uint32_t dqpn;

  printf("exchange info");
  exchange_info(sock, &ctx, &rkey, &raddr, &dqpn);

  printf("info exchange success\n");
  printf("dqpn: %d\n", dqpn);
  setup_qp(&ctx, dqpn);

  memset(ctx.buffer, 'a', msg_len);

  struct ibv_sge sge = {
      .addr = (uint64_t)ctx.buffer, .length = msg_len, .lkey = ctx.mr->lkey};
  struct ibv_send_wr wr = {.wr_id = 7,
			   .sg_list = &sge,
			   .num_sge = 1,
			   .imm_data = 11,
			   .opcode = IBV_WR_SEND,
			   .send_flags = IBV_SEND_SIGNALED};
  wr.wr.rdma.remote_addr = raddr;
  wr.wr.rdma.rkey = rkey;

  struct ibv_send_wr *bad_wr;
  handshake(sock);

  ibv_post_send(ctx.qp, &wr, &bad_wr);
  struct ibv_wc wc;
  while (ibv_poll_cq(ctx.cq, 1, &wc) < 1) {
    usleep(1000);
  }

  close(sock);
}

int main(int argc, char *argv[]) {
  if (argc == 2) {
    int msg_len = atoi(argv[1]);
    run_server(msg_len);
  } else if (argc == 3) {
    int msg_len = atoi(argv[1]);
    run_client(msg_len, argv[2]);
  } else {
    fprintf(stderr, "Usage: %s <msg_len> [server_ip]\n", argv[0]);
    fprintf(stderr, "  Run without arguments to start as server\n");
    fprintf(stderr, "  Run with server_ip to connect as client\n");
    return EXIT_FAILURE;
  }
  sleep(1);

  return EXIT_SUCCESS;
}
