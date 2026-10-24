from PIL import Image
import sys
im=Image.open(sys.argv[1]).convert("RGB");w,h=im.size
n=sum(1 for y in range(0,h,3) for x in range(0,w,3) if (lambda r,g,b:r>60 and b>50 and g<r*0.72 and g<b*0.8 and r+g+b>140)(*im.getpixel((x,y))))
print(n)
