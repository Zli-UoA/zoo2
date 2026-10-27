import { test } from "node:test";
import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { ZooApp, verifySignature, type Delivery } from "./index.ts";

test("verifies raw HTTP signatures and rejects stale or modified payloads", async () => {
 const timestamp="1793059200",body='{"event":"hello"}',secret="test signing secret";
 const signature=`v1=${createHmac("sha256",secret).update(`v1:${timestamp}:${body}`).digest("hex")}`;
 const now=Number(timestamp)*1000;
 assert.equal(await verifySignature(secret,timestamp,body,signature,now),true);
 assert.equal(await verifySignature(secret,timestamp,`${body} `,signature,now),false);
 assert.equal(await verifySignature(secret,timestamp,body,signature,now+301000),false);
});
test("deduplicates concurrent deliveries and retries failed handlers", async () => {
 const client=new ZooApp({baseUrl:"https://zoo.example",credentials:{access_token:"access",refresh_token:"refresh"}});
 const delivery:Delivery={delivery_id:"delivery",ack_token:"ack",attempt:1,event:{id:"event",version:1,type:"message.created",actor_id:"user",data:{}}};
 let calls=0;
 const handler=async()=>{calls++;await Promise.resolve();};
 await Promise.all([client.dispatch(delivery,handler),client.dispatch(delivery,handler)]);
 await client.dispatch(delivery,handler);assert.equal(calls,1);
 const second={...delivery,event:{...delivery.event,id:"second"}};
 await assert.rejects(client.dispatch(second,async()=>{throw new Error("retry me");}));
 await client.dispatch(second,handler);assert.equal(calls,2);
});
test("refreshes credentials and retains the message id across a retry",async()=>{
 const messages:Record<string,unknown>[]=[];let saved=false;
 const fetcher:typeof fetch=async(input,init)=>{
  if(String(input).endsWith("/oauth/token"))return Response.json({access_token:"new",refresh_token:"rotated"});
  const body=JSON.parse(String(init?.body));messages.push(body);
  if(messages.length===1)return Response.json({error:"expired"},{status:401});
  assert.equal(new Headers(init?.headers).get("authorization"),"Bearer new");
  return Response.json({data:{id:body.id},txid:"42"});
 };
 const client=new ZooApp({baseUrl:"https://zoo.example",credentials:{access_token:"old",refresh_token:"refresh"},fetch:fetcher,saveCredentials:async()=>{saved=true;}});
 const message=await client.postMessage({channel_id:"channel",content:"hello"});
 assert.equal(messages[0].id,messages[1].id);assert.equal(message.id,messages[0].id);assert.equal(saved,true);
});
